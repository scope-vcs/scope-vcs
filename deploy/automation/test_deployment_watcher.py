import copy
from datetime import datetime, timedelta
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, patch

import deployment_watcher as watcher


NOW = "2026-09-23T00:01:00Z"
BEFORE = "2026-09-22T23:00:00Z"


def release(run_id=123, status="completed", conclusion="failure", created_at=NOW, attempt=1):
    return {"id": run_id, "status": status, "conclusion": conclusion,
            "created_at": created_at, "run_attempt": attempt,
            "head_branch": "main", "event": "workflow_dispatch",
            "repository": {"full_name": "scope-vcs/scope-vcs"},
            "path": ".github/workflows/release.yml"}


class WatcherTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.client = MagicMock()
        self.projects = [{"id": watcher.PROJECT_ID, "workspaceRoot": str(watcher.CHECKOUT)}]
        self.agent = {"status": "running", "activeRunId": "run-1", "updatedAt": NOW, "pendingRuntimeRequest": None}
        self.client.shell.side_effect = self.shell
        self.t3 = MagicMock()
        self.t3.return_value.__enter__.return_value = self.client
        self.runs = []
        self.by_id = {}
        self.mocks = {}
        replacements = {
            "STATE_DIR": self.root, "STATE_PATH": self.root / "supervision.json",
            "stamp": lambda: NOW, "T3Client": self.t3,
            "create_worktree": MagicMock(return_value="commit"),
            "alert": MagicMock(return_value="issue-url"),
            "jobs": MagicMock(return_value=[]), "github": MagicMock(side_effect=self.github),
        }
        for name, value in replacements.items():
            self.mocks[name] = value
            patcher = patch.object(watcher, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        watcher.persist({"installed_at": BEFORE, "listed_through": BEFORE, "runs": {}, "threads": {}})

    def github(self, path):
        if path.startswith("actions/workflows/release.yml/runs?"):
            return {"workflow_runs": copy.deepcopy(self.runs)}
        if path.startswith("actions/runs/"):
            return copy.deepcopy(self.by_id[int(path.split("/")[-1])])
        raise AssertionError(f"Unexpected GitHub request: {path}")

    def shell(self):
        threads = [{"id": info["thread_id"], **self.agent} for info in self.saved()["threads"].values()]
        return {"projects": self.projects, "threads": threads, "archivedThreads": []}

    def saved(self):
        return json.loads(watcher.STATE_PATH.read_text())

    def starts(self):
        return [call.args[0] for call in self.client.dispatch.call_args_list
                if call.args[0]["type"] == "message.dispatch"]

    def test_healthy_release_is_observed_without_an_agent_until_it_fails(self):
        self.runs = [release(status="queued", conclusion=None)]
        result = watcher.poll()
        self.assertEqual(result["active_releases"], 1)
        self.assertEqual(self.saved()["runs"]["123"]["status"], "watching")
        self.assertEqual(self.starts(), [])
        self.runs = [release()]
        watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")
        self.assertEqual(len(self.starts()), 1)

    def test_escalation_keeps_a_running_release_active_until_github_confirms_completion(self):
        self.runs = [release(status="in_progress", conclusion=None)]
        watcher.poll()
        watcher.stop_owned("deadline_exceeded")
        self.runs = []
        self.by_id[123] = release(status="in_progress", conclusion=None)
        self.assertEqual(watcher.poll()["active_releases"], 1)
        self.assertEqual(self.saved()["runs"]["123"]["status"], "escalated")
        self.by_id[123] = release(status="completed", conclusion="failure")
        self.assertEqual(watcher.poll()["active_releases"], 0)
        self.assertEqual(self.starts(), [])

    def test_initialization_excludes_historical_completed_releases(self):
        watcher.STATE_PATH.unlink()
        self.runs = [release(1, "completed", "failure", BEFORE),
                     release(2, "completed", "success", BEFORE),
                     release(3, "in_progress", created_at=BEFORE)]
        watcher.poll(initialize=True)
        self.assertEqual(set(self.saved()["runs"]), {"3"})
        self.assertEqual(len(self.starts()), 0)

    def test_operator_quarantine_preserves_escalation_and_reopens_on_new_activity(self):
        old = "2026-08-01T00:00:00Z"
        run = release(status="queued", conclusion=None, created_at=old) | {
            "updated_at": old, "run_started_at": old, "head_sha": "a" * 40}
        self.by_id[123] = run
        state = self.saved()
        state["runs"]["123"] = {"run_id": 123, "attempt": 1, "status": "escalated",
                                 "created_at": old, "workflow_status": "queued"}
        watcher.persist(state)
        watcher.quarantine_run(123, "Operator accepted stale GitHub queue")
        quarantined = self.saved()
        self.assertEqual(quarantined["runs"]["123"]["status"], "escalated")
        self.assertEqual(watcher.poll()["active_releases"], 0)
        self.assertEqual(self.starts(), [])
        for changed, new_jobs in (({"run_attempt": 2}, []), ({"status": "in_progress"}, []),
                                  ({"updated_at": NOW}, []), ({"head_sha": "b" * 40}, []),
                                  ({"run_started_at": NOW}, []), ({}, [{"name": "Plan selected components"}])):
            with self.subTest(changed=changed, new_jobs=new_jobs):
                watcher.persist(copy.deepcopy(quarantined))
                self.by_id[123] = run | changed
                self.mocks["jobs"].return_value = new_jobs
                self.assertEqual(watcher.poll()["active_releases"], 1)
                record = self.saved()["runs"]["123"]
                self.assertEqual(record["status"], "watching")
                self.assertEqual(record["quarantine"]["lifted_at"], NOW)
                self.assertEqual(self.starts(), [])
        record["status"] = "escalated"
        state = self.saved()
        state["runs"]["123"] = record
        watcher.persist(state)
        self.by_id[123] = run
        self.mocks["jobs"].return_value = []
        watcher.quarantine_run(123, "Operator renewed stale GitHub queue disposition")
        renewed = self.saved()["runs"]["123"]
        self.assertEqual(renewed["quarantine_history"], [record["quarantine"]])
        self.assertNotIn("lifted_at", renewed["quarantine"])

    def test_completed_lifted_quarantine_does_not_require_historical_github_run(self):
        state = self.saved()
        state["runs"]["123"] = {"run_id": 123, "attempt": 1, "status": "verified",
                                 "created_at": BEFORE, "workflow_status": "completed",
                                 "quarantine": {"at": BEFORE, "lifted_at": NOW}}
        watcher.persist(state)
        self.assertEqual(watcher.poll()["active_releases"], 0)
        self.assertEqual(self.saved()["runs"]["123"]["status"], "verified")
        self.assertEqual(self.starts(), [])

    def test_quarantine_rejects_live_jobs_recent_activity_or_repair_ownership(self):
        old = "2026-08-01T00:00:00Z"
        run = release(status="queued", conclusion=None, created_at=old) | {
            "updated_at": old, "run_started_at": old, "head_sha": "a" * 40}
        base = self.saved()
        base["runs"]["123"] = {"run_id": 123, "attempt": 1, "status": "escalated", "created_at": old}
        for change, jobs, thread in (({"status": "in_progress"}, [], {}), ({"updated_at": NOW}, [], {}),
                                     ({"run_attempt": 2}, [], {}), ({"head_branch": "feature"}, [], {}),
                                     ({}, [{"name": "Plan selected components"}], {}),
                                     ({}, [], {"status": "escalated", "owns_agent": True})):
            with self.subTest(change=change, jobs=jobs, thread=thread):
                state = copy.deepcopy(base)
                if thread:
                    state["threads"]["incident"] = thread
                watcher.persist(state)
                self.by_id[123] = run | change
                self.mocks["jobs"].return_value = jobs
                with self.assertRaises(ValueError):
                    watcher.quarantine_run(123, "Operator accepted stale GitHub queue")
                self.assertNotIn("quarantine", self.saved()["runs"]["123"])

    def test_initialization_admits_late_retry_of_historical_run(self):
        watcher.STATE_PATH.unlink()
        self.runs = [release(1, "completed", "failure", BEFORE),
                     release(2, "completed", "failure", BEFORE, attempt=2)
                     | {"run_started_at": NOW}]
        watcher.poll(initialize=True)
        state = self.saved()
        self.assertEqual(set(state["runs"]), {"2"})
        self.assertEqual(state["runs"]["2"]["attempt"], 2)
        self.assertEqual(state["runs"]["2"]["attempt_started_at"], NOW)
        self.assertEqual(state["runs"]["2"]["status"], "monitoring")
        self.assertEqual(len(self.starts()), 1)

    def test_webhook_admits_old_run_retry_outside_the_recent_run_window(self):
        self.by_id[123] = release(123, "completed", "failure", BEFORE, attempt=2) | {"run_started_at": NOW}
        watcher.poll(run_ids=[123])
        self.assertEqual(self.saved()["runs"]["123"]["attempt"], 2)
        self.assertEqual(len(self.starts()), 1)

    def test_success_requires_release_verification_job(self):
        self.runs = [release(status="completed", conclusion="success")]
        watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")
        self.mocks["jobs"].return_value = [{"name": "Verify and record release", "conclusion": "success"}]
        watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "verified")
        self.agent = {"status": "completed", "activeRunId": None, "updatedAt": NOW}
        self.assertEqual(watcher.stop_owned("deadline_exceeded"), 0)
        self.assertEqual(next(iter(self.saved()["threads"].values()))["status"], "verified")

    def test_uncertain_dispatch_reuses_persisted_command(self):
        self.runs = [release()]
        def lose_response(command):
            if command["type"] == "message.dispatch":
                raise TimeoutError("Accepted but response lost")
        self.client.dispatch.side_effect = lose_response
        with self.assertRaisesRegex(RuntimeError, "t3.repair-supervision: TimeoutError"):
            watcher.poll()
        pending = next(iter(self.saved()["threads"].values()))["pending_command"]
        self.client.dispatch.side_effect = None
        watcher.poll()
        self.assertEqual(self.starts(), [pending, pending])
        self.assertNotIn("pending_command", next(iter(self.saved()["threads"].values())))

    def test_healthy_progress_does_not_restart_every_poll(self):
        self.runs = [release()]
        watcher.poll()
        for minute in range(1, 6):
            at = (datetime.fromisoformat(NOW.replace("Z", "+00:00"))
                  + timedelta(minutes=minute)).isoformat()
            self.agent["updatedAt"] = at
            with patch.object(watcher, "stamp", return_value=at):
                watcher.poll()
        self.assertEqual(len(self.starts()), 1)
        self.assertEqual(next(iter(self.saved()["threads"].values()))["recoveries"], 0)

    def test_project_mismatch_fails_before_dispatch(self):
        self.runs = [release()]
        self.projects = []
        with self.assertRaises(RuntimeError):
            watcher.poll()
        self.client.dispatch.assert_not_called()

    def test_github_failure_prevents_repair_dispatch(self):
        self.mocks["github"].side_effect = RuntimeError("unavailable")
        with self.assertRaises(RuntimeError):
            watcher.poll()
        self.client.dispatch.assert_not_called()

    def test_verified_correction_receipt_recovers_original(self):
        self.runs = [release(status="completed", conclusion="failure")]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"corrections": {"123": 456}, "blocker": False}))
        self.by_id[456] = release(456, "completed", "success")
        self.mocks["jobs"].return_value = [{"name": "Verify and record release", "conclusion": "success"}]
        watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "recovered")
        self.assertEqual(self.saved()["runs"]["123"]["corrected_by"], 456)
        self.assertEqual(self.saved()["runs"]["456"]["status"], "verified")

    def test_failed_correction_chain_resolves_to_final_verified_run_idempotently(self):
        self.runs = [release(status="completed", conclusion="failure")]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"corrections": {"123": 456, "456": 789}, "blocker": False}))
        self.by_id[456] = release(456, "completed", "failure", "2026-09-23T00:02:00Z")
        self.by_id[789] = release(789, "completed", "success", "2026-09-23T00:03:00Z")
        watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")
        self.mocks["jobs"].return_value = [{"name": "Verify and record release", "conclusion": "success"}]
        watcher.poll()
        first = self.saved()
        for run_id in ("123", "456"):
            self.assertEqual(first["runs"][run_id]["status"], "recovered")
            self.assertEqual(first["runs"][run_id]["corrected_by"], 789)
        self.assertEqual(first["runs"]["789"]["status"], "verified")
        watcher.poll()
        self.assertEqual(self.saved()["runs"], first["runs"])

    def test_correction_chain_requires_final_production_verification(self):
        self.runs = [release(status="completed", conclusion="failure")]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"corrections": {"123": 456, "456": 789}, "blocker": False}))
        self.by_id[456] = release(456, "completed", "failure", "2026-09-23T00:02:00Z")
        self.by_id[789] = release(789, "completed", "success", "2026-09-23T00:03:00Z")
        self.mocks["jobs"].return_value = [{"name": "Verify and record release", "conclusion": "failure"}]
        watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")
        self.assertEqual(self.saved()["runs"]["456"]["status"], "monitoring")

    def test_receipt_adopts_unassigned_completed_correction(self):
        self.runs = [release(status="completed", conclusion="failure")]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        self.runs = [release(456, "completed", "success", "2026-09-23T00:02:00Z"), self.runs[0]]
        self.mocks["jobs"].return_value = [{"name": "Verify and record release", "conclusion": "success"}]
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"corrections": {"123": 456}, "blocker": False}))
        self.by_id[456] = self.runs[0]
        watcher.poll()
        state = self.saved()
        self.assertEqual(state["runs"]["123"]["status"], "recovered")
        self.assertEqual(state["runs"]["456"]["incident_id"], info["incident_id"])

    def test_cycle_receipt_does_not_resolve_or_add_runs(self):
        self.runs = [release(status="completed", conclusion="failure")]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"corrections": {"123": 456, "456": 123}, "blocker": False}))
        self.by_id[456] = release(456, "completed", "failure")
        self.by_id[123] = self.runs[0]
        watcher.poll()
        self.assertEqual(set(self.saved()["runs"]), {"123"})
        self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")

    def test_failed_retry_of_final_correction_reopens_chain(self):
        self.runs = [release(status="completed", conclusion="failure")]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"corrections": {"123": 456}, "blocker": False}))
        self.by_id[456] = release(456, "completed", "success", "2026-09-23T00:02:00Z")
        self.mocks["jobs"].return_value = [{"name": "Verify and record release", "conclusion": "success"}]
        watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "recovered")
        self.by_id[456] = release(456, "completed", "failure", "2026-09-23T00:02:00Z", attempt=2)
        self.mocks["jobs"].return_value = []
        watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")

    def test_failed_retry_of_closed_correction_reopens_recovered_original(self):
        self.runs = [release(status="completed", conclusion="failure")]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"corrections": {"123": 456}, "blocker": False}))
        self.by_id[456] = release(456, "completed", "success", "2026-09-23T00:02:00Z")
        self.mocks["jobs"].return_value = [{"name": "Verify and record release", "conclusion": "success"}]
        self.agent = {"status": "completed", "activeRunId": None, "updatedAt": NOW}
        watcher.poll()
        self.assertEqual(self.saved()["threads"][info["incident_id"]]["status"], "verified")
        self.assertEqual(self.saved()["runs"]["123"]["status"], "recovered")
        retry = release(456, "completed", "failure", "2026-09-23T00:02:00Z", attempt=2)
        self.runs = [release(status="completed", conclusion="failure"), retry]
        self.by_id[456] = retry
        self.mocks["jobs"].return_value = []
        watcher.poll()
        state = self.saved()
        self.assertEqual(state["runs"]["123"]["status"], "monitoring")
        self.assertNotIn("corrected_by", state["runs"]["123"])
        self.assertEqual(state["runs"]["123"]["incident_id"], state["runs"]["456"]["incident_id"])
        self.assertNotEqual(state["runs"]["123"]["incident_id"], info["incident_id"])

    def test_failed_correction_cannot_close_original(self):
        self.runs = [release(status="completed", conclusion="failure")]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"corrections": {"123": 456}, "blocker": False}))
        self.by_id[456] = release(456, "completed", "failure")
        watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")
        self.assertEqual(self.saved()["runs"]["456"]["incident_id"], info["incident_id"])

    def test_stale_correction_cannot_close_new_original_attempt(self):
        self.runs = [release(status="completed", conclusion="failure")]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"corrections": {"123": 456}, "blocker": False}))
        corrected_at = "2026-09-23T00:02:00Z"
        retried_at = "2026-09-23T00:03:00Z"
        self.by_id[456] = release(456, "completed", "success", corrected_at)
        self.mocks["jobs"].return_value = [{"name": "Verify and record release", "conclusion": "success"}]
        watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "recovered")
        self.runs = [release(status="completed", conclusion="failure", attempt=2)
                     | {"run_started_at": retried_at}]
        watcher.poll()
        record = self.saved()["runs"]["123"]
        self.assertEqual(record["attempt"], 2)
        self.assertEqual(record["attempt_started_at"], retried_at)
        self.assertEqual(record["status"], "monitoring")
        self.assertEqual(len(self.starts()), 1)

    def test_initial_prompt_includes_all_grouped_releases(self):
        self.runs = [release(123), release(456)]
        watcher.poll()
        self.assertEqual(len(self.starts()), 1)
        prompt = self.starts()[0]["text"]
        self.assertIn("/runs/123", prompt)
        self.assertIn("/runs/456", prompt)

    def test_new_release_does_not_launch_until_previous_agent_stops(self):
        self.runs = [release()]
        watcher.poll()
        self.runs = [release(456), release(123, "completed", "success")]
        self.mocks["jobs"].return_value = [{"name": "Verify and record release", "conclusion": "success"}]
        watcher.poll()
        self.assertEqual(len(self.starts()), 1, "A previous live agent still owns deployment mutations")

    def test_later_release_reaches_existing_agents_inbox(self):
        self.runs = [release()]
        watcher.poll()
        self.runs = [release(456), release()]
        watcher.poll()
        self.assertEqual(len(self.starts()), 1)
        info = next(iter(self.saved()["threads"].values()))
        inbox = json.loads(watcher.inbox_path(info).read_text())
        self.assertEqual({run["run_id"] for run in inbox["releases"]}, {123, 456})
        self.assertIn(str(watcher.inbox_path(info)), self.starts()[0]["text"])

    def test_escalated_agent_must_stop_before_next_investigation(self):
        self.runs = [release()]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"corrections": {}, "blocker": True}))
        watcher.poll()
        self.assertEqual(self.saved()["threads"][info["incident_id"]]["status"], "escalated")
        self.runs = [release(456), release()]
        watcher.poll()
        self.assertEqual(len(self.starts()), 1)
        self.assertEqual(self.saved()["runs"]["456"]["status"], "waiting")
        self.agent = {"status": "interrupted", "activeRunId": None, "updatedAt": NOW}
        watcher.poll()
        self.assertEqual(len(self.starts()), 1)
        watcher.poll()
        self.assertEqual(len(self.starts()), 2)
        self.assertNotEqual(self.saved()["runs"]["456"]["incident_id"], info["incident_id"])
        self.mocks["alert"].assert_called_once_with(
            123, "approval_required", recoveries=0,
            thread_id=info["thread_id"], provider="claudeAgent")

    def test_pending_dispatch_remains_idempotent_after_release_finishes(self):
        self.runs = [release()]
        self.client.dispatch.side_effect = [None, TimeoutError("Lost response")]
        with self.assertRaisesRegex(RuntimeError, "t3.repair-supervision: TimeoutError"):
            watcher.poll()
        self.client.dispatch.side_effect = None
        self.runs = [release(status="completed", conclusion="success")]
        self.mocks["jobs"].return_value = [{"name": "Verify and record release", "conclusion": "success"}]
        watcher.poll()
        self.assertEqual(len(self.starts()), 2)
        self.assertEqual(self.starts()[0], self.starts()[1])

    def test_interrupt_reaches_a_run_whose_id_appears_later(self):
        self.runs = [release()]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        interrupts = lambda: [call.args[0] for call in self.client.dispatch.call_args_list
                              if call.args[0]["type"] == "run.interrupt"]
        past_deadline = "2026-09-23T04:02:00Z"
        self.agent = {"status": "starting", "activeRunId": None, "updatedAt": NOW}
        with patch.object(watcher, "stamp", return_value=past_deadline):
            watcher.poll()
        self.assertEqual(self.saved()["threads"][info["incident_id"]]["stop_reason"], "deadline_exceeded")
        self.assertEqual(interrupts(), [])
        self.agent = {"status": "running", "activeRunId": "run-2", "updatedAt": NOW}
        with patch.object(watcher, "stamp", return_value=past_deadline):
            watcher.poll()
        self.assertEqual(interrupts(), [watcher.interrupt_command(
            f"{info['incident_id']}-stop-0-run-2", info["thread_id"], "run-2")])

    def test_agent_thread_missing_from_shell_escalates(self):
        self.runs = [release()]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        self.client.shell.side_effect = lambda: {"projects": self.projects, "threads": [], "archivedThreads": []}
        watcher.poll()
        self.assertEqual(self.saved()["threads"][info["incident_id"]]["status"], "escalated")
        self.mocks["alert"].assert_called_once_with(
            123, "agent_unavailable", recoveries=0, thread_id=info["thread_id"], provider="claudeAgent")

    def test_failed_agent_read_keeps_repair_ownership(self):
        self.runs = [release()]
        watcher.poll()
        self.client.shell.side_effect = RuntimeError("T3 unavailable")
        with self.assertRaises(RuntimeError):
            watcher.poll()
        self.assertTrue(next(iter(self.saved()["threads"].values()))["owns_agent"])

    def test_cached_stop_survives_release_lookup_failure_and_retains_owner_until_confirmed(self):
        self.runs = [release()]
        watcher.poll()
        self.mocks["github"].side_effect = RuntimeError("GitHub lookup unavailable")
        self.assertEqual(watcher.stop_owned("attempts_exhausted"), 1)
        self.assertEqual(self.client.dispatch.call_args.args[0]["type"], "run.interrupt")
        self.agent = {"status": "interrupted", "activeRunId": None, "updatedAt": NOW}
        self.assertEqual(watcher.stop_owned("attempts_exhausted"), 0)
        self.assertEqual(len(self.starts()), 1)

    def test_failed_escalation_publication_cannot_resume_a_stopped_repair(self):
        self.runs = [release()]
        watcher.poll()
        self.agent = {"status": "interrupted", "activeRunId": None, "updatedAt": NOW}
        self.mocks["alert"].side_effect = RuntimeError("Issue API unavailable")
        with self.assertRaises(RuntimeError):
            watcher.stop_owned("attempts_exhausted")
        info = next(iter(self.saved()["threads"].values()))
        self.assertEqual(info["status"], "escalated")
        self.assertIn("pending_alert_run_id", info)
        self.mocks["alert"].side_effect = None
        watcher.poll()
        self.assertEqual(len(self.starts()), 1)
        self.assertNotIn("pending_alert_run_id", next(iter(self.saved()["threads"].values())))

    def test_untrusted_correction_keeps_original_open_and_supervised(self):
        self.runs = [release(status="completed", conclusion="failure")]
        watcher.poll()
        self.client.shell.reset_mock()
        info = next(iter(self.saved()["threads"].values()))
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"corrections": {"123": 456}, "blocker": False}))
        self.by_id[456] = release(456, "completed", "success") | {"head_branch": "other"}
        watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")
        self.client.shell.assert_called_once_with()
        self.mocks["alert"].assert_not_called()

    def test_temporary_malformed_receipt_does_not_interrupt_supervision(self):
        self.runs = [release(status="completed", conclusion="failure")]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        for content in ["{", "[]", '{"corrections": []}', '{"blocker": "yes"}']:
            with self.subTest(receipt=content):
                self.client.shell.reset_mock()
                path.write_text(content)
                watcher.poll()
                self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")
                self.client.shell.assert_called_once_with()
                self.mocks["alert"].assert_not_called()
        path.write_text(json.dumps({"corrections": {}, "blocker": False}))
        with patch.object(watcher, "stamp", return_value="2026-09-23T00:02:00Z"):
            watcher.poll()
        self.assertNotIn("receipt_error_at", self.saved()["threads"][info["incident_id"]])
        with patch.object(watcher, "stamp", return_value="2026-09-23T00:04:00Z"):
            watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")
        self.mocks["alert"].assert_not_called()

    def test_persistently_malformed_receipt_escalates_after_grace(self):
        self.runs = [release(status="completed", conclusion="failure")]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("{")
        watcher.poll()
        with patch.object(watcher, "stamp", return_value="2026-09-23T00:02:59Z"):
            watcher.poll()
        self.mocks["alert"].assert_not_called()
        with patch.object(watcher, "stamp", return_value="2026-09-23T00:03:00Z"):
            watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "escalated")
        self.mocks["alert"].assert_called_once_with(
            123, "verification_failed", recoveries=0,
            thread_id=info["thread_id"], provider="claudeAgent")
        self.assertEqual(self.client.dispatch.call_args.args[0],
                         watcher.interrupt_command(f"{info['incident_id']}-stop-0-run-1", info["thread_id"], "run-1"))


if __name__ == "__main__":
    unittest.main()
