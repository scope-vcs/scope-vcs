import copy
import json
import sys
import unittest
from decimal import Decimal
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from journal import LOCK_SECONDS, integers
from lifecycle import Broker
from protocol import Denied, Pending, request
from provider import Provider, ProviderRejected


ATTEMPT = "attempt_" + "a" * 32
TOKEN = "scope_bootstrap_" + "b" * 64
IMAGE = "registry.example.org/team/custom@sha256:" + "c" * 64
START = {"action": "start", "attempt_id": ATTEMPT, "bootstrap_token": TOKEN}
STOP = {"action": "stop", "attempt_id": ATTEMPT}


class MemoryJournal:
    def __init__(self):
        self.records = {}
        self.fail_phase = None

    def acquire(self, attempt, owner, now):
        value = self.records.setdefault(attempt, {"attempt_id": attempt})
        if value.get("lock_until", 0) >= now:
            raise Pending("locked")
        value.update(lock_owner=owner, lock_until=now + LOCK_SECONDS)
        return copy.deepcopy(value)

    def save(self, record, owner):
        if self.records[record["attempt_id"]]["lock_owner"] != owner:
            raise Pending("owner changed")
        if record["phase"] == self.fail_phase:
            self.fail_phase = None
            raise OSError("lost journal write")
        self.records[record["attempt_id"]] = copy.deepcopy(record)

    def release(self, attempt, owner):
        record = self.records[attempt]
        if record.get("lock_owner") == owner:
            record.pop("lock_owner", None)
            record.pop("lock_until", None)


class Authority:
    def __init__(self):
        self.start_allowed = True
        self.stop_allowed = False
        self.calls = []
        self.image = IMAGE
        self.after_authorize = None

    def authorize(self, command):
        self.calls.append(copy.deepcopy(command))
        if not (self.start_allowed if command["action"] == "start" else self.stop_allowed):
            raise Denied("denied")
        if self.after_authorize:
            self.after_authorize()
        result = {"action": command["action"], "attempt_id": command["attempt_id"]}
        if command["action"] == "start":
            result.update(image=self.image, deadline_unix=10000)
        return result


class FakeProvider:
    def __init__(self):
        self.prepares = 0
        self.launches = 0
        self.cleanups = 0
        self.tasks = []
        self.stopped = "stopped"
        self.lose_launch_response = False
        self.lose_prepare_response = False
        self.lose_cleanup_response = False
        self.reject_launch = False
        self.on_prepare = None

    def prepare(self, record, token):
        self.prepares += 1
        if self.on_prepare:
            self.on_prepare()
        if self.lose_prepare_response:
            raise OSError("lost registration response")
        return {"taskDefinition": "definition:" + str(self.prepares), "count": 1}

    def launch(self, specification):
        self.launches += 1
        if self.reject_launch:
            raise ProviderRejected("rejected")
        self.tasks = ["task:1"]
        if self.lose_launch_response:
            raise OSError("lost launch response")
        return self.tasks[0]

    def find(self, attempt):
        return self.tasks

    def stop(self, task):
        return self.stopped

    def cleanup(self, record):
        self.cleanups += 1
        if self.lose_cleanup_response:
            raise OSError("lost cleanup response")


class LifecycleTests(unittest.TestCase):
    def setUp(self):
        self.journal = MemoryJournal()
        self.authority = Authority()
        self.provider = FakeProvider()
        self.now = 100
        self.broker = Broker(self.journal, self.authority, self.provider, lambda: self.now)

    def call(self, command=START):
        return self.broker.handle(command, "invocation")

    def record(self):
        return self.journal.records[ATTEMPT]

    def test_custom_image_launch_uses_authoritative_image(self):
        self.assertEqual(self.call(), {"status": "started", "task_arn": "task:1"})
        self.assertEqual(self.record()["image"], IMAGE)
        self.assertNotIn(TOKEN, json.dumps(self.record()))

    def test_replay_after_runtime_claim_does_not_consume_or_reauthorize_bootstrap(self):
        self.call()
        self.authority.start_allowed = False
        self.assertEqual(self.call()["status"], "started")
        self.assertEqual(self.provider.launches, 1)
        self.assertEqual(len(self.authority.calls), 2)

    def test_forged_bootstrap_cannot_replay_known_attempt(self):
        self.call()
        forged = dict(START, bootstrap_token="scope_bootstrap_" + "d" * 64)
        self.assertEqual(self.call(forged)["status"], "ambiguous")
        self.assertEqual(self.provider.launches, 1)

    def test_denied_attempt_creates_no_aws_resources(self):
        self.authority.start_allowed = False
        self.assertEqual(self.call()["status"], "rejected")
        self.assertEqual(self.provider.prepares, 0)
        self.assertEqual(self.provider.launches, 0)

    def test_cancel_after_setup_prevents_launch(self):
        self.provider.on_prepare = lambda: setattr(self.authority, "start_allowed", False)
        self.assertEqual(self.call()["status"], "ambiguous")
        self.assertEqual(self.provider.launches, 0)
        self.authority.stop_allowed = True
        self.now += 301
        self.assertEqual(self.call(STOP)["status"], "stopped")

    def test_concurrent_request_cannot_take_live_lock(self):
        self.journal.acquire(ATTEMPT, "another", self.now)
        self.assertEqual(self.call()["status"], "ambiguous")
        self.assertEqual(self.provider.prepares, 0)
        self.assertEqual(self.record()["lock_owner"], "another")

    def test_crashed_invocation_lock_expires_after_lambda_hard_timeout(self):
        self.journal.acquire(ATTEMPT, "crashed", self.now)
        self.now += 121
        self.assertEqual(self.call()["status"], "ambiguous")
        self.now += 60
        self.assertEqual(self.call()["status"], "started")

    def test_lost_run_response_is_discovered_without_relaunch(self):
        self.provider.lose_launch_response = True
        self.assertEqual(self.call()["status"], "ambiguous")
        self.assertEqual(self.record()["phase"], "launching")
        self.assertEqual(self.call()["status"], "started")
        self.assertEqual(self.provider.launches, 1)

    def test_lost_success_journal_write_is_discovered_without_relaunch(self):
        self.journal.fail_phase = "running"
        self.assertEqual(self.call()["status"], "ambiguous")
        self.assertEqual(self.record()["phase"], "launching")
        self.assertEqual(self.call()["status"], "started")
        self.assertEqual(self.provider.launches, 1)

    def test_unknown_launch_is_never_reissued_after_ecs_token_expiry(self):
        self.provider.lose_launch_response = True
        self.call()
        self.provider.tasks = []
        self.now += 90000
        self.assertEqual(self.call()["status"], "ambiguous")
        self.assertEqual(self.provider.launches, 1)

    def test_unknown_launch_absence_waits_for_consistency(self):
        self.provider.lose_launch_response = True
        self.call()
        self.provider.tasks = []
        self.authority.stop_allowed = True
        self.assertEqual(self.call(STOP)["status"], "ambiguous")
        self.assertEqual(self.provider.cleanups, 0)
        self.now += 301
        self.assertEqual(self.call(STOP)["status"], "stopped")
        self.assertEqual(self.call()["status"], "rejected")
        self.assertEqual(self.provider.launches, 1)

    def test_lost_registration_response_retains_cleanup_ownership(self):
        self.provider.lose_prepare_response = True
        self.assertEqual(self.call()["status"], "ambiguous")
        self.authority.stop_allowed = True
        self.assertEqual(self.call(STOP)["status"], "ambiguous")
        self.now += 301
        self.assertEqual(self.call(STOP)["status"], "stopped")
        self.assertEqual(self.provider.launches, 0)

    def test_missing_durable_launch_marker_prevents_aws_launch(self):
        self.journal.fail_phase = "launching"
        self.assertEqual(self.call()["status"], "ambiguous")
        self.assertEqual(self.provider.launches, 0)
        self.assertEqual(self.call()["status"], "started")
        self.assertEqual(self.provider.prepares, 1)

    def test_attempt_image_change_is_denied_on_resumption(self):
        self.journal.fail_phase = "launching"
        self.call()
        self.authority.image = "other.example/job@sha256:" + "e" * 64
        self.assertEqual(self.call()["status"], "ambiguous")
        self.assertEqual(self.provider.launches, 0)

    def test_stop_requires_independent_authorization(self):
        self.call()
        self.assertEqual(self.call(STOP)["status"], "ambiguous")
        self.assertEqual(self.provider.cleanups, 0)
        self.assertEqual(self.record()["phase"], "running")

    def test_stop_confirmation_is_required_before_cleanup(self):
        self.call()
        self.authority.stop_allowed = True
        self.provider.stopped = "stopping"
        self.assertEqual(self.call(STOP)["status"], "ambiguous")
        self.assertEqual(self.provider.cleanups, 0)
        self.assertEqual(self.call()["status"], "ambiguous")
        self.provider.stopped = "stopped"
        self.assertEqual(self.call(STOP)["status"], "stopped")

    def test_known_recent_task_missing_from_ecs_cannot_finish_cleanup(self):
        self.call()
        self.authority.stop_allowed = True
        self.provider.tasks = []
        self.provider.stopped = "missing"
        self.assertEqual(self.call(STOP)["status"], "ambiguous")
        self.assertEqual(self.provider.cleanups, 0)
        self.assertEqual(self.record()["phase"], "stopping")
        self.now += 301
        self.assertEqual(self.call(STOP)["status"], "stopped")

    def test_cleanup_failure_remains_retryable_without_launch(self):
        self.call()
        self.authority.stop_allowed = True
        self.provider.lose_cleanup_response = True
        self.assertEqual(self.call(STOP)["status"], "ambiguous")
        self.provider.lose_cleanup_response = False
        self.assertEqual(self.call(STOP)["status"], "stopped")
        self.assertEqual(self.call()["status"], "rejected")
        self.assertEqual(self.provider.launches, 1)

    def test_stop_before_start_leaves_permanent_tombstone(self):
        self.authority.stop_allowed = True
        self.assertEqual(self.call(STOP)["status"], "stopped")
        self.now += 90000
        self.assertEqual(self.call()["status"], "rejected")
        self.assertEqual(self.provider.launches, 0)

    def test_ecs_rejection_requires_cleanup_before_safe_rejection(self):
        self.provider.reject_launch = True
        self.assertEqual(self.call()["status"], "rejected")
        self.assertEqual(self.provider.cleanups, 1)
        self.assertEqual(self.record()["phase"], "stopped")


class SchemaTests(unittest.TestCase):
    def test_caller_cannot_supply_aws_instructions(self):
        for field in ("image", "taskRoleArn", "executionRoleArn", "secret_arn", "subnets", "cpu", "deadline_unix", "task_arn"):
            with self.subTest(field=field), self.assertRaises(Denied):
                request(dict(START, **{field: "substitution"}))
        with self.assertRaises(Denied):
            request(dict(STOP, task_arn="other-attempt"))

    def test_identity_and_token_are_bounded(self):
        for attempt in ("", ATTEMPT + "/other", "attempt_other", 12):
            with self.assertRaises(Denied):
                request(dict(START, attempt_id=attempt))
        for token in ("", "scope_attempt_" + "a" * 64, "x" * 100000):
            with self.assertRaises(Denied):
                request(dict(START, bootstrap_token=token))

    def test_dynamodb_numbers_restore_ecs_integer_inputs(self):
        self.assertEqual(integers({"request": {"count": Decimal(1)}}), {"request": {"count": 1}})
        self.assertIs(type(integers(Decimal(1))), int)


class RecordingAws:
    def __init__(self):
        self.calls = []

    def create_secret(self, **kwargs):
        self.calls.append(("create", kwargs))
        return {"ARN": "arn:bound-bootstrap"}

    def register_task_definition(self, **kwargs):
        self.calls.append(("register", kwargs))
        return {"taskDefinition": {"taskDefinitionArn": "arn:bound-definition"}}


class ProviderTests(unittest.TestCase):
    def test_task_role_secret_network_and_limits_are_constructed_by_broker(self):
        aws = RecordingAws()
        settings = SimpleNamespace(cluster="arn:cluster/scope-vcs-production-runner", region="us-east-1", subnets=["subnet-owned"], security_group="sg-owned", execution_role="role-owned", log_group="logs-owned", api="https://api.example.org", registry_secret="")
        provider = Provider(aws, aws, settings)
        spec = provider.prepare({"attempt_id": ATTEMPT, "image": IMAGE, "deadline_unix": 1000}, TOKEN)
        definition = aws.calls[1][1]
        self.assertNotIn("taskRoleArn", definition)
        self.assertEqual(definition["executionRoleArn"], "role-owned")
        self.assertEqual((definition["cpu"], definition["memory"]), ("4096", "16384"))
        container = definition["containerDefinitions"][0]
        self.assertEqual(container["image"], IMAGE)
        self.assertEqual(container["secrets"], [{"name": "SCOPE_BOOTSTRAP_TOKEN", "valueFrom": "arn:bound-bootstrap"}])
        self.assertNotIn("repositoryCredentials", container)
        self.assertEqual(spec["networkConfiguration"]["awsvpcConfiguration"]["subnets"], ["subnet-owned"])
        self.assertEqual(spec["startedBy"], ATTEMPT)
        self.assertNotIn(TOKEN, json.dumps(spec))
        settings.registry_secret = "arn:registry-exact"
        settings.registry_host = "registry.example.org"
        provider.prepare({"attempt_id": ATTEMPT, "image": IMAGE, "deadline_unix": 1000}, TOKEN)
        container = aws.calls[-1][1]["containerDefinitions"][0]
        self.assertEqual(container["repositoryCredentials"], {"credentialsParameter": "arn:registry-exact"})
        self.assertNotIn("arn:registry-exact", json.dumps(container["secrets"]))
        provider.prepare({"attempt_id": ATTEMPT, "image": "unrelated.example/team/job@sha256:" + "c" * 64, "deadline_unix": 1000}, TOKEN)
        self.assertNotIn("repositoryCredentials", aws.calls[-1][1]["containerDefinitions"][0])


if __name__ == "__main__":
    unittest.main()
