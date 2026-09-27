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


def release(run_id=123, status="queued", conclusion=None, created_at=NOW, attempt=1):
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
        self.client.request.return_value = {
            "projects": [{"id": watcher.PROJECT_ID, "workspaceRoot": str(watcher.CHECKOUT)}],
            "threads": [],
        }
        self.client.thread.return_value = {
            "latestTurn": {"state": "running", "startedAt": NOW},
            "session": {"status": "running"}, "messages": [], "activities": [],
        }
        self.t3 = MagicMock()
        self.t3.return_value.__enter__.return_value = self.client
        self.runs = []
        self.by_id = {}
        self.mocks = {}
        replacements = {
            "STATE_DIR": self.root, "STATE_PATH": self.root / "supervision.json",
            "stamp": lambda: NOW, "T3Client": self.t3,
            "create_worktree": MagicMock(return_value="commit"),
            "heartbeat": MagicMock(), "alert": MagicMock(return_value="issue-url"),
            "jobs": MagicMock(return_value=[]), "github": MagicMock(side_effect=self.github),
        }
        for name, value in replacements.items():
            self.mocks[name] = value
            patcher = patch.object(watcher, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        scheduler = patch.object(watcher.deployment_scheduler, "poll", return_value={})
        scheduler.start()
        self.addCleanup(scheduler.stop)
        watcher.persist({"installed_at": BEFORE, "listed_through": BEFORE, "runs": {}, "threads": {}})

    def github(self, path):
        if path.startswith("actions/workflows/release.yml/runs?"):
            return {"workflow_runs": copy.deepcopy(self.runs)}
        if path.startswith("actions/runs/"):
            return copy.deepcopy(self.by_id[int(path.split("/")[-1])])
        raise AssertionError(f"Unexpected GitHub request: {path}")

    def saved(self):
        return json.loads(watcher.STATE_PATH.read_text())

    def starts(self):
        return [call.args[0] for call in self.client.dispatch.call_args_list
                if call.args[0]["type"] == "thread.turn.start"]

    def test_queued_and_early_failed_releases_are_admitted_at_midnight(self):
        for status, conclusion in [("queued", None), ("completed", "failure")]:
            with self.subTest(status=status):
                self.runs = [release(status=status, conclusion=conclusion)]
                watcher.poll()
                self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")
        self.assertEqual(len(self.starts()), 1)
        self.assertEqual(self.mocks["heartbeat"].call_count, 2)

    def test_initialization_excludes_historical_completed_releases(self):
        watcher.STATE_PATH.unlink()
        self.runs = [release(1, "completed", "failure", BEFORE),
                     release(2, "completed", "success", BEFORE),
                     release(3, "in_progress", created_at=BEFORE)]
        watcher.poll(initialize=True)
        self.assertEqual(set(self.saved()["runs"]), {"3"})
        self.assertEqual(len(self.starts()), 1)

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

    def test_success_requires_release_verification_job(self):
        self.runs = [release(status="completed", conclusion="success")]
        watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")
        self.mocks["jobs"].return_value = [{"name": "Verify and record release", "conclusion": "success"}]
        watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "verified")

    def test_uncertain_dispatch_reuses_persisted_command(self):
        self.runs = [release()]
        def lose_response(command):
            if command["type"] == "thread.turn.start":
                raise TimeoutError("Accepted but response lost")
        self.client.dispatch.side_effect = lose_response
        with self.assertRaises(TimeoutError):
            watcher.poll()
        self.mocks["heartbeat"].assert_not_called()
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
            self.client.thread.return_value["activities"] = [{"createdAt": at}]
            with patch.object(watcher, "stamp", return_value=at):
                watcher.poll()
        self.assertEqual(len(self.starts()), 1)
        self.assertEqual(next(iter(self.saved()["threads"].values()))["recoveries"], 0)

    def test_project_mismatch_fails_before_dispatch_and_heartbeat(self):
        self.runs = [release()]
        self.client.request.return_value["projects"] = []
        with self.assertRaises(RuntimeError):
            watcher.poll()
        self.client.dispatch.assert_not_called()
        self.mocks["heartbeat"].assert_not_called()

    def test_github_failure_does_not_renew_heartbeat(self):
        self.mocks["github"].side_effect = RuntimeError("unavailable")
        with self.assertRaises(RuntimeError):
            watcher.poll()
        self.mocks["heartbeat"].assert_not_called()

    def test_scheduler_failure_still_supervises_release_without_heartbeat(self):
        self.runs = [release()]
        with patch.object(watcher.deployment_scheduler, "poll", side_effect=RuntimeError("unavailable")):
            with self.assertRaisesRegex(RuntimeError, "Daily release scheduler failed"):
                watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")
        self.assertEqual(len(self.starts()), 1)
        self.mocks["heartbeat"].assert_not_called()

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
        self.client.thread.return_value = {"latestTurn": {"state": "completed", "startedAt": NOW},
                                           "session": {"status": "idle"}, "messages": [], "activities": []}
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
        prompt = self.starts()[0]["message"]["text"]
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
        self.assertIn(str(watcher.inbox_path(info)), self.starts()[0]["message"]["text"])

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
        self.client.thread.return_value = {"latestTurn": {"state": "interrupted"}, "session": {"status": "ready"}}
        watcher.poll()
        self.assertEqual(len(self.starts()), 1)
        watcher.poll()
        self.assertEqual(len(self.starts()), 2)
        self.assertNotEqual(self.saved()["runs"]["456"]["incident_id"], info["incident_id"])
        self.mocks["alert"].assert_called_once_with(
            123, "approval_required", recoveries=0,
            thread_id=info["thread_id"], provider="codex")

    def test_pending_dispatch_remains_idempotent_after_release_finishes(self):
        self.runs = [release()]
        self.client.dispatch.side_effect = [None, TimeoutError("Lost response")]
        with self.assertRaises(TimeoutError):
            watcher.poll()
        self.client.dispatch.side_effect = None
        self.runs = [release(status="completed", conclusion="success")]
        self.mocks["jobs"].return_value = [{"name": "Verify and record release", "conclusion": "success"}]
        watcher.poll()
        self.assertEqual(len(self.starts()), 2)
        self.assertEqual(self.starts()[0], self.starts()[1])

    def test_failed_agent_read_does_not_publish_heartbeat(self):
        self.runs = [release()]
        watcher.poll()
        self.mocks["heartbeat"].reset_mock()
        self.client.thread.side_effect = RuntimeError("T3 unavailable")
        with self.assertRaises(RuntimeError):
            watcher.poll()
        self.mocks["heartbeat"].assert_not_called()

    def test_untrusted_correction_keeps_original_open_and_supervised(self):
        self.runs = [release(status="completed", conclusion="failure")]
        watcher.poll()
        self.mocks["heartbeat"].reset_mock()
        info = next(iter(self.saved()["threads"].values()))
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"corrections": {"123": 456}, "blocker": False}))
        self.by_id[456] = release(456, "completed", "success") | {"head_branch": "other"}
        watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")
        self.client.thread.assert_called_once_with(info["thread_id"])
        self.mocks["heartbeat"].assert_called_once()
        self.mocks["alert"].assert_not_called()

    def test_temporary_malformed_receipt_does_not_interrupt_supervision(self):
        self.runs = [release(status="completed", conclusion="failure")]
        watcher.poll()
        info = next(iter(self.saved()["threads"].values()))
        path = watcher.receipt_path(info)
        path.parent.mkdir(parents=True, exist_ok=True)
        for content in ["{", "[]", '{"corrections": []}', '{"blocker": "yes"}']:
            with self.subTest(receipt=content):
                self.mocks["heartbeat"].reset_mock()
                self.client.thread.reset_mock()
                path.write_text(content)
                watcher.poll()
                self.assertEqual(self.saved()["runs"]["123"]["status"], "monitoring")
                self.client.thread.assert_called_once_with(info["thread_id"])
                self.mocks["heartbeat"].assert_called_once()
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
        self.mocks["heartbeat"].reset_mock()
        with patch.object(watcher, "stamp", return_value="2026-09-23T00:03:00Z"):
            watcher.poll()
        self.assertEqual(self.saved()["runs"]["123"]["status"], "escalated")
        self.mocks["alert"].assert_called_once_with(
            123, "verification_failed", recoveries=0,
            thread_id=info["thread_id"], provider="codex")
        self.mocks["heartbeat"].assert_called_once()
        self.assertEqual(self.client.dispatch.call_args.args[0]["type"], "thread.turn.interrupt")


if __name__ == "__main__":
    unittest.main()
