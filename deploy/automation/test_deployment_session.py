from datetime import datetime, timedelta, timezone
import fcntl
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import Mock, patch

import deployment_session as session
from deployment_diagnostics import OperationFailure, diagnostic, operation
from deployment_runtime import save_json
from heartbeat import session_health


NOW = datetime(2026, 10, 8, 7, 8, tzinfo=timezone.utc)
TRUSTED = {"id": 123, "run_attempt": 2, "head_branch": "main", "event": "workflow_dispatch",
           "repository": {"full_name": "scope-vcs/scope-vcs"}, "path": ".github/workflows/release.yml"}
PAYLOAD = {"event": "workflow_run", "action": "completed", "repository": "scope-vcs/scope-vcs",
           "run_id": 123, "attempt": 2, "delivery": "12345678-1234-1234-1234-123456789abc"}


class Clock(datetime):
    current = NOW

    @classmethod
    def now(cls, tz=None):
        return cls.current


class SessionTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        Clock.current = NOW
        self.launch = Mock()
        self.poll = Mock(return_value={"active_releases": 0, "repair_owners": 0})
        self.daily = Mock(return_value={"intents": {}, "last_audited_on": "2026-10-07"})
        self.pins = Mock(return_value=False)
        self.heartbeat = Mock()
        self.stop = Mock(return_value=0)
        self.issue = Mock(return_value="assigned-escalation")
        replacements = [(session, "STATE_DIR", self.root), (session, "launch", self.launch),
                        (session, "datetime", Clock), (session, "github", Mock(return_value=TRUSTED)),
                        (session, "heartbeat", self.heartbeat), (session.watcher, "poll", self.poll),
                        (session, "ensure_issue", self.issue),
                        (session.watcher, "stop_owned", self.stop),
                        (session.scheduler, "poll", self.daily), (session.pins, "poll", self.pins)]
        for owner, name, value in replacements:
            patcher = patch.object(owner, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        save_json(self.root / "scheduler-owner.json", {"owner": "t3", "activated_on": "2026-10-08"})
        save_json(self.root / "supervision.json", {"installed_at": NOW.isoformat(), "runs": {}, "threads": {}})

    def test_duplicate_webhook_and_crashed_launch_keep_one_durable_trigger(self):
        self.launch.side_effect = RuntimeError("launch failed")
        with self.assertRaises(RuntimeError):
            session.trigger("webhook", PAYLOAD | {"action": "in_progress"}, NOW)
        self.launch.side_effect = None
        session.trigger("webhook", PAYLOAD | {"action": "in_progress"}, NOW)
        result = session.run()
        self.assertEqual(result["status"], "idle")
        self.assertEqual(len(session.read("triggers.json")["requests"]), 1)
        self.poll.assert_called_once_with(run_ids=[123])
        save_json(self.root / "supervision.json", {"runs": {"123": {"attempt": 2, "status": "verified"}}, "threads": {}})
        self.assertEqual(session.trigger("webhook", PAYLOAD, NOW), {"deduplicated": "release-123-2"})
        self.assertEqual(self.launch.call_count, 2)
        for workflow_status in ("in_progress", "completed", None):
            record = {"attempt": 2, "status": "escalated", "created_at": NOW.isoformat()}
            if workflow_status is not None:
                record["workflow_status"] = workflow_status
            save_json(self.root / "supervision.json", {"installed_at": NOW.isoformat(), "runs": {"123": record}, "threads": {}})
            queue = session.read("triggers.json")
            queue["requests"]["release-123-2"].update(events=["2:in_progress"], handled_at=NOW.isoformat())
            save_json(self.root / "triggers.json", queue)
            self.launch.reset_mock()
            result = session.trigger("webhook", PAYLOAD, NOW + timedelta(hours=5))
            with self.subTest(workflow_status=workflow_status):
                self.assertEqual("deduplicated" in result, workflow_status == "completed")
                self.assertEqual(self.launch.call_count, int(workflow_status != "completed"))
                self.assertEqual(len(session.read("triggers.json")["requests"]), 1)
        save_json(self.root / "session.json", {"id": "expired", "status": "failed", "started_at": NOW.isoformat(),
                                             "deadline_at": (NOW + timedelta(hours=4)).isoformat()})
        Clock.current = NOW + timedelta(hours=5)
        result = session.run()
        self.assertEqual(result["status"], "idle")
        self.poll.assert_called_with(run_ids=[123])
        self.launch.reset_mock()
        self.assertEqual(session.trigger("webhook", PAYLOAD, Clock.current), {"deduplicated": "release-123-2"})
        self.launch.assert_not_called()

    def test_events_verify_github_identity_and_use_the_current_attempt(self):
        for mismatch in ({"head_branch": "feature"}, {"path": ".github/workflows/validate.yml"},
                         {"repository": {"full_name": "other/repo"}}, {"run_attempt": 1}):
            with self.subTest(mismatch=mismatch), patch.object(session, "github", return_value=TRUSTED | mismatch):
                self.assertEqual(session.trigger("webhook", PAYLOAD, NOW), {"ignored": True})
        session.trigger("webhook", PAYLOAD | {"attempt": 1}, NOW)
        self.assertEqual(set(session.read("triggers.json")["requests"]), {"release-123-2"})
        self.launch.assert_called_once()
        session.run()
        result = session.trigger("webhook", PAYLOAD, NOW)
        self.assertEqual(result["queued"], "release-123-2")
        self.assertEqual(self.launch.call_count, 2)

    def test_quarantine_replay_rechecks_github_and_publishes_its_disposition(self):
        old = "2026-09-01T00:00:00Z"
        run = TRUSTED | {"status": "queued", "head_sha": "a" * 40, "created_at": old,
                         "updated_at": old, "run_started_at": old}
        quarantine = {"at": "2026-10-07T00:00:00Z", "reason": "Operator accepted stale queue", "run": {
            "id": 123, "run_attempt": 2, "status": "queued", "head_sha": "a" * 40,
            "created_at": old, "updated_at": old, "run_started_at": old}}
        record = {"run_id": 123, "attempt": 2, "status": "escalated", "created_at": old,
                  "workflow_status": "queued", "quarantine": quarantine}
        save_json(self.root / "supervision.json", {"installed_at": NOW.isoformat(), "runs": {"123": record}, "threads": {}})
        with patch.object(session, "github", return_value=run), patch.object(session.watcher, "jobs", return_value=[]):
            session.trigger("webhook", PAYLOAD, NOW)
            session.run()
            self.assertEqual(session.trigger("webhook", PAYLOAD, NOW), {"deduplicated": "release-123-2"})
        published = self.heartbeat.call_args.kwargs["status"]
        self.assertIn("123", published["quarantined"])
        self.assertNotIn("reason", published["quarantined"]["123"])
        self.launch.reset_mock()
        for change, jobs in (({"updated_at": NOW.isoformat()}, []), ({}, [{"name": "Plan selected components"}])):
            with self.subTest(change=change, jobs=jobs):
                queue = session.read("triggers.json")
                queue["requests"]["release-123-2"]["handled_at"] = NOW.isoformat()
                save_json(self.root / "triggers.json", queue)
                with patch.object(session, "github", return_value=run | change), patch.object(session.watcher, "jobs", return_value=jobs):
                    result = session.trigger("webhook", PAYLOAD, NOW)
                self.assertEqual(result["queued"], "release-123-2")
                self.assertNotIn("handled_at", session.read("triggers.json")["requests"]["release-123-2"])
        self.assertEqual(self.launch.call_count, 2)

    def test_operator_quarantine_publishes_stopped_session_without_another_trigger(self):
        old = "2026-09-01T00:00:00Z"
        run = TRUSTED | {"status": "queued", "head_sha": "a" * 40, "created_at": old,
                         "updated_at": old, "run_started_at": old}
        record = {"run_id": 123, "attempt": 2, "status": "escalated", "created_at": old,
                  "workflow_status": "queued"}
        save_json(self.root / "supervision.json", {"installed_at": NOW.isoformat(), "runs": {"123": record}, "threads": {}})
        save_json(self.root / "session.json", {"id": "stopped", "status": "escalated", "phase": "stopped",
                                             "releases": 1, "repair_owners": 0, "progress_at": NOW.isoformat()})
        with patch.object(session.watcher, "STATE_DIR", self.root), \
                patch.object(session.watcher, "STATE_PATH", self.root / "supervision.json"), \
                patch.object(session.watcher, "github", return_value=run), \
                patch.object(session.watcher, "jobs", return_value=[]), \
                patch.object(session.watcher, "stamp", return_value=NOW.isoformat()), \
                patch("sys.argv", ["session", "quarantine", "123", "--reason", "Operator accepted stale GitHub queue"]), \
                patch("builtins.print"):
            session.main()
            session.main()
        published = self.heartbeat.call_args.kwargs["status"]
        self.assertEqual(published["id"], "stopped")
        self.assertEqual(published["status"], "escalated")
        self.assertEqual(published["releases"], 0)
        self.assertEqual(published["quarantined"]["123"]["run"]["head_sha"], "a" * 40)
        self.assertNotIn("reason", published["quarantined"]["123"])
        self.assertEqual(session.read("session.json")["quarantined"], published["quarantined"])
        self.assertEqual(self.heartbeat.call_count, 2)
        self.launch.assert_not_called()

    def test_one_process_owns_supervision_and_never_dispatches_a_daily_for_a_manual_event(self):
        session.trigger("webhook", PAYLOAD, NOW)
        with (self.root / "watcher.lock").open("a") as owner:
            fcntl.flock(owner, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaisesRegex(RuntimeError, "Another supervision owner"):
                session.run()
        self.poll.assert_not_called()
        session.run()
        self.daily.assert_called_once_with(NOW, dispatch_day=None)

    def test_crash_restart_keeps_deadline_and_does_not_redispatch_processed_daily_intent(self):
        session.trigger("daily", now=NOW)
        self.poll.return_value = {"active_releases": 1, "repair_owners": 0}
        with patch.object(session.time, "sleep", side_effect=RuntimeError("process interrupted")):
            with self.assertRaises(RuntimeError):
                session.run()
        before = session.read("session.json")
        self.assertEqual(before["status"], "failed")
        self.assertIn("handled_at", session.read("triggers.json")["requests"]["daily-2026-10-08"])
        Clock.current += timedelta(minutes=30)
        self.poll.return_value = {"active_releases": 0, "repair_owners": 0}
        result = session.run()
        self.assertEqual((result["id"], result["deadline_at"]), (before["id"], before["deadline_at"]))
        self.assertEqual(self.daily.call_args.kwargs, {"dispatch_day": None})

    def test_session_waits_for_stop_confirmation_and_finishes_at_its_deadline(self):
        session.trigger("reconcile", now=NOW)
        self.poll.return_value = {"active_releases": 0, "repair_owners": 1}
        self.stop.return_value = 1
        def advance(_):
            Clock.current += timedelta(hours=2, minutes=1)
            if Clock.current >= NOW + timedelta(hours=4):
                session.trigger("daily", now=Clock.current)
        with patch.object(session.time, "sleep", side_effect=advance):
            result = session.run()
        self.assertEqual(result["status"], "escalated")
        self.assertEqual(result["repair_owners"], 1)
        self.stop.assert_called_once_with("deadline_exceeded")
        self.assertEqual(self.poll.call_count, 2)
        self.assertNotIn("handled_at", session.read("triggers.json")["requests"]["daily-2026-10-08"])

    def test_repeated_dependency_failures_exhaust_the_saved_process_recovery_budget(self):
        cases = ((failing, new_work) for failing in (self.heartbeat, self.poll) for new_work in (False, True))
        for failing, new_work in cases:
            with self.subTest(heartbeat_failure=failing is self.heartbeat, new_work=new_work):
                for name in ("session.json", "triggers.json"):
                    (self.root / name).unlink(missing_ok=True)
                for mock in (self.heartbeat, self.poll, self.daily, self.pins, self.stop, self.issue):
                    mock.reset_mock(side_effect=True)
                Clock.current = NOW
                for kind in ("daily", "pins", "webhook", "reconcile"):
                    session.trigger(kind, PAYLOAD if kind == "webhook" else None, NOW)
                initial = set(session.read("triggers.json")["requests"])
                failing.side_effect = RuntimeError("dependency unavailable")
                for _ in range(4):
                    with self.assertRaises(RuntimeError):
                        session.run()
                previous = session.read("session.json")
                failing.side_effect = None
                Clock.current += timedelta(minutes=2)
                new = session.trigger("reconcile", now=Clock.current)["queued"] if new_work else None
                result = session.run()
                self.stop.assert_called_once_with("attempts_exhausted")
                self.issue.assert_called_once()
                queue = session.read("triggers.json")["requests"]
                for key in initial:
                    self.assertIn("handled_at", queue[key])
                    receipt = queue[key]["retirements"][previous["id"]]
                    self.assertEqual(receipt["reason"], "attempts_exhausted")
                    self.assertEqual(receipt["alert_url"], "assigned-escalation")
                if new_work:
                    self.assertIn("handled_at", queue[new])
                    self.assertNotIn("retirements", queue[new])
                    self.assertEqual(result["status"], "idle")
                    self.assertNotEqual(result["id"], previous["id"])
                    self.daily.assert_called_with(Clock.current, dispatch_day=None)
                    self.pins.assert_called_once_with(release_open=False, now=Clock.current, requested_weeks=())
                else:
                    self.assertEqual(result["status"], "escalated")
                    self.assertEqual(result["id"], previous["id"])
                    self.pins.assert_not_called()
                retired = next(call.kwargs["status"] for call in self.heartbeat.call_args_list
                               if call.kwargs["status"].get("phase") == "stopped")
                self.assertEqual(retired["escalated_requests"], 4)
                self.assertFalse(session_health(json.dumps(retired), Clock.current, 1200))

    def test_failed_escalation_preserves_unhandled_work_and_blocks_new_budget_until_retirement(self):
        session.trigger("daily", now=NOW)
        self.heartbeat.side_effect = RuntimeError("heartbeat unavailable")
        for _ in range(4):
            with self.assertRaises(RuntimeError):
                session.run()
        previous = session.read("session.json")
        self.heartbeat.side_effect = None
        self.issue.side_effect = RuntimeError("issue unavailable")
        new = session.trigger("reconcile", now=NOW)["queued"]
        result = session.run()
        self.assertEqual(result["phase"], "stopping")
        self.assertEqual(result["failure"]["operation"], "github.trigger-escalation")
        self.assertEqual(result["id"], previous["id"])
        self.poll.assert_not_called()
        queue = session.read("triggers.json")["requests"]
        self.assertTrue(all("handled_at" not in value for value in queue.values()))
        self.assertFalse(session_health(json.dumps(result), NOW, 1200))
        self.issue.side_effect = None
        result = session.run()
        self.assertEqual(result["status"], "idle")
        self.assertNotEqual(result["id"], previous["id"])
        queue = session.read("triggers.json")["requests"]
        self.assertIn("retirements", queue["daily-2026-10-08"])
        self.assertNotIn("retirements", queue[new])
        self.daily.assert_called_once_with(NOW, dispatch_day=None)

    def test_completion_arriving_during_a_poll_remains_pending_until_another_poll(self):
        session.trigger("webhook", PAYLOAD | {"action": "in_progress"}, NOW)
        outcomes = []
        def poll(**kwargs):
            if not outcomes:
                session.trigger("webhook", PAYLOAD, NOW)
            outcomes.append(kwargs)
            return {"active_releases": 0, "repair_owners": 0}
        self.poll.side_effect = poll
        during = []
        with patch.object(session.time, "sleep", side_effect=lambda _: during.append(session.read("triggers.json"))):
            result = session.run()
        self.assertEqual(result["status"], "idle")
        self.assertEqual(len(outcomes), 2)
        self.assertEqual(len(during), 1)
        self.assertNotIn("handled_at", during[0]["requests"]["release-123-2"])
        request = session.read("triggers.json")["requests"]["release-123-2"]
        self.assertEqual(request["events"], ["2:in_progress", "2:completed"])
        self.assertIn("handled_at", request)

    def test_expiry_publishes_confirmed_stop_and_preserves_failed_cleanup_health(self):
        for owners in (0, 1):
            with self.subTest(owners=owners):
                Clock.current = NOW
                self.heartbeat.reset_mock()
                published_states = []
                self.heartbeat.side_effect = lambda **kwargs: published_states.append(json.loads(json.dumps(kwargs["status"])))
                save_json(self.root / "session.json", {})
                self.poll.return_value = {"active_releases": 0, "repair_owners": 1}
                self.stop.return_value = owners
                def expire(_):
                    Clock.current += timedelta(hours=4, minutes=1)
                with patch.object(session.time, "sleep", side_effect=expire):
                    result = session.run()
                published = published_states[-1]
                self.assertEqual(published["status"], "escalated")
                self.assertEqual(published["repair_owners"], owners)
                self.assertEqual(bool(published.get("failure")), bool(owners))
                self.assertEqual(published["installed_at"], NOW.isoformat())
                self.assertEqual(published["phase"], "stopped" if not owners else "stopping")
                self.assertEqual(session.read("session.json"), result)

    def test_daily_reconciliation_failure_keeps_supervising_repair_without_successful_progress(self):
        session.trigger("daily", now=NOW)
        self.daily.side_effect = RuntimeError("daily API unavailable")
        self.poll.return_value = {"active_releases": 1, "repair_owners": 1}
        during = []
        def recover(_):
            during.append((session.read("session.json"), session.read("triggers.json")))
            Clock.current += timedelta(minutes=1)
            self.daily.side_effect = None
            self.poll.return_value = {"active_releases": 0, "repair_owners": 0}
        with patch.object(session.time, "sleep", side_effect=recover):
            result = session.run()
        self.assertEqual(self.poll.call_count, 2)
        self.stop.assert_not_called()
        self.assertEqual(during[0][0]["progress_at"], NOW.isoformat())
        self.assertEqual(during[0][0]["failure"]["operation"], "github.daily-dispatch")
        self.assertNotIn("handled_at", during[0][1]["requests"]["daily-2026-10-08"])
        self.assertEqual((result["status"], result["recoveries"]), ("idle", 0))
        self.assertNotIn("failure", result)

    def test_exhausted_recovery_stops_and_confirms_existing_repair_before_new_work(self):
        save_json(self.root / "session.json", {"id": "prior", "status": "failed", "started_at": "2026-10-08T06:00:00+00:00",
                                             "deadline_at": "2026-10-08T10:00:00+00:00", "recoveries": 3})
        save_json(self.root / "supervision.json", {"installed_at": NOW.isoformat(), "runs": {}, "threads": {"repair": {"owns_agent": True}}})
        session.trigger("daily", now=NOW)
        outcomes = iter((1, 0))
        def stop(reason):
            owners = next(outcomes)
            save_json(self.root / "supervision.json", {"installed_at": NOW.isoformat(), "runs": {}, "threads": {"repair": {"owns_agent": bool(owners)}}})
            return owners
        self.stop.side_effect = stop
        with patch.object(session.time, "sleep"):
            result = session.run()
        self.assertEqual(self.stop.call_count, 2)
        self.assertEqual(result["status"], "idle")
        self.assertNotEqual(result["id"], "prior")
        self.assertEqual(self.daily.call_args.kwargs, {"dispatch_day": "2026-10-08"})

    def test_expired_failed_session_cannot_consume_the_new_daily_dispatch(self):
        save_json(self.root / "session.json", {"id": "yesterday", "status": "failed", "started_at": "2026-10-07T07:08:00+00:00",
                                             "deadline_at": "2026-10-07T11:08:00+00:00"})
        session.trigger("daily", now=NOW)
        result = session.run()
        self.stop.assert_called_once_with("deadline_exceeded")
        self.assertEqual(self.daily.call_args.kwargs, {"dispatch_day": "2026-10-08"})
        self.assertEqual(result["deadline_at"], "2026-10-08T11:08:00+00:00")

    def test_daily_arriving_during_expiry_stays_queued_until_the_old_repair_stops(self):
        Clock.current = NOW - timedelta(hours=4, minutes=2)
        session.trigger("reconcile", now=Clock.current)
        self.poll.return_value = {"active_releases": 0, "repair_owners": 1}
        before = {}
        def advance(_):
            if not before:
                before.update(session.read("session.json"))
                Clock.current = NOW + timedelta(minutes=1)
                session.trigger("daily", now=NOW)
            else:
                Clock.current += timedelta(seconds=30)
        self.stop.side_effect = [1, 0]
        def stopped():
            self.poll.return_value = {"active_releases": 0, "repair_owners": 0}
            return self.stop()
        with patch.object(session.time, "sleep", side_effect=advance), \
                patch.object(session.watcher, "stop_owned", side_effect=lambda reason: stopped()):
            result = session.run()
        self.assertEqual(self.stop.call_count, 2)
        self.assertEqual(result["status"], "idle")
        self.assertNotEqual(result["id"], before["id"])
        self.assertEqual(result["recoveries"], 0)
        self.assertEqual(self.daily.call_args.kwargs, {"dispatch_day": "2026-10-08"})
        self.assertEqual(sum(call.kwargs["dispatch_day"] is not None for call in self.daily.call_args_list), 1)
        self.assertIn("handled_at", session.read("triggers.json")["requests"]["daily-2026-10-08"])

    def test_cutover_guard_rejects_triggers_before_ownership_changes(self):
        save_json(self.root / "scheduler-owner.json", {"owner": "systemd"})
        with self.assertRaisesRegex(RuntimeError, "cutover"):
            session.trigger("daily", now=NOW)
        self.launch.assert_not_called()
        self.assertFalse((self.root / "triggers.json").exists())

    def test_early_manual_task_invocation_cannot_consume_the_future_daily_or_weekly_slot(self):
        self.assertEqual(session.trigger("daily", now=NOW - timedelta(minutes=1)), {"ignored": "not-due"})
        monday = datetime(2026, 10, 5, 13, 59, tzinfo=timezone.utc)
        self.assertEqual(session.trigger("pins", now=monday), {"ignored": "not-due"})
        self.assertFalse((self.root / "triggers.json").exists())
        session.trigger("daily", now=NOW)
        self.assertIn("daily-2026-10-08", session.read("triggers.json")["requests"])

    def test_diagnostics_name_the_operation_and_redact_raw_failure_text(self):
        with self.assertRaises(OperationFailure) as failure:
            with operation("github.release-jobs"):
                raise RuntimeError("Authorization: secret-value, provider response")
        value = diagnostic(failure.exception)
        self.assertEqual(value, {"operation": "github.release-jobs", "category": "RuntimeError"})
        self.assertNotIn("secret-value", str(failure.exception))


class ProcessDeadlineTests(unittest.TestCase):
    def test_blocked_operation_is_interrupted_by_the_independent_process_deadline(self):
        deadline = datetime.now(timezone.utc) + timedelta(seconds=0.05)
        with self.assertRaises(session.SessionDeadlineExceeded):
            with session.process_deadline(deadline):
                time.sleep(0.5)

    def test_deadline_inside_maintenance_start_escapes_its_retry_handler(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(session.pins, "STATE_PATH", Path(directory) / "pins.json"), \
                patch.object(session.pins, "start", side_effect=lambda *args: time.sleep(0.5)) as start:
            session.pins.poll(True, datetime(2026, 10, 5, 14, tzinfo=timezone.utc), requested_weeks=("2026-W41",))
            with self.assertRaises(session.SessionDeadlineExceeded):
                with session.process_deadline(datetime.now(timezone.utc) + timedelta(seconds=0.05)):
                    session.pins.poll(False, datetime(2026, 10, 5, 14, tzinfo=timezone.utc))
            start.assert_called_once()
            intent = json.loads(session.pins.STATE_PATH.read_text())["weeks"]["2026-W41"]
            self.assertNotIn("first_attempt_at", intent)


if __name__ == "__main__":
    unittest.main()
