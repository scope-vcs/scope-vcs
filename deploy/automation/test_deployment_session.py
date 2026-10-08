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
        replacements = [(session, "STATE_DIR", self.root), (session, "launch", self.launch),
                        (session, "datetime", Clock), (session, "github", Mock(return_value=TRUSTED)),
                        (session, "heartbeat", self.heartbeat), (session.watcher, "poll", self.poll),
                        (session.watcher, "stop_owned", self.stop),
                        (session.scheduler, "poll", self.daily), (session.pins, "poll", self.pins)]
        for owner, name, value in replacements:
            patcher = patch.object(owner, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        save_json(self.root / "scheduler-owner.json", {"owner": "t3", "activated_on": "2026-10-08"})
        save_json(self.root / "supervision.json", {"runs": {}, "threads": {}})

    def test_duplicate_webhook_and_crashed_launch_keep_one_durable_trigger(self):
        self.launch.side_effect = RuntimeError("launch failed")
        with self.assertRaises(RuntimeError):
            session.trigger("webhook", PAYLOAD, NOW)
        self.launch.side_effect = None
        session.trigger("webhook", PAYLOAD, NOW)
        result = session.run()
        self.assertEqual(result["status"], "idle")
        self.assertEqual(len(session.read("triggers.json")["requests"]), 1)
        self.poll.assert_called_once_with(expired=False, run_ids=[123])
        save_json(self.root / "supervision.json", {"runs": {"123": {"attempt": 2, "status": "verified"}}, "threads": {}})
        self.assertEqual(session.trigger("webhook", PAYLOAD, NOW), {"deduplicated": "release-123-2"})
        self.assertEqual(self.launch.call_count, 2)

    def test_events_verify_github_identity_and_use_the_current_attempt(self):
        for mismatch in ({"head_branch": "feature"}, {"path": ".github/workflows/validate.yml"},
                         {"repository": {"full_name": "other/repo"}}, {"run_attempt": 1}):
            with self.subTest(mismatch=mismatch), patch.object(session, "github", return_value=TRUSTED | mismatch):
                self.assertEqual(session.trigger("webhook", PAYLOAD, NOW), {"ignored": True})
        session.trigger("webhook", PAYLOAD | {"attempt": 1}, NOW)
        self.assertEqual(set(session.read("triggers.json")["requests"]), {"release-123-2"})
        self.launch.assert_called_once()

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
        def advance(_):
            Clock.current += timedelta(hours=2, minutes=3)
        with patch.object(session.time, "sleep", side_effect=advance):
            result = session.run()
        self.assertEqual(result["status"], "escalated")
        self.assertEqual(result["repair_owners"], 1)
        self.assertTrue(self.poll.call_args.kwargs["expired"])
        self.assertEqual(self.poll.call_count, 3)

    def test_repeated_dependency_failures_exhaust_the_saved_process_recovery_budget(self):
        session.trigger("reconcile", now=NOW)
        self.poll.side_effect = RuntimeError("dependency unavailable")
        for _ in range(4):
            with self.assertRaises(RuntimeError):
                session.run()
        result = session.run()
        self.assertEqual(result["status"], "escalated")
        self.stop.assert_called_once_with("attempts_exhausted")
        self.assertEqual(self.poll.call_count, 4)

    def test_exhausted_recovery_stops_and_confirms_existing_repair_before_new_work(self):
        save_json(self.root / "session.json", {"id": "prior", "status": "failed", "started_at": "2026-10-08T06:00:00+00:00",
                                             "deadline_at": "2026-10-08T10:00:00+00:00", "recoveries": 3})
        save_json(self.root / "supervision.json", {"runs": {}, "threads": {"repair": {"owns_agent": True}}})
        session.trigger("daily", now=NOW)
        self.stop.side_effect = [1, 0]
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


if __name__ == "__main__":
    unittest.main()
