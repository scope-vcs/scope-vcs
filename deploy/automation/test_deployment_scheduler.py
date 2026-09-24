from datetime import datetime
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import deployment_scheduler as scheduler


def utc(value):
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


class SchedulerTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = Path(directory.name) / "daily-dispatch.json"
        state_path = patch.object(scheduler, "STATE_PATH", self.path)
        state_path.start()
        self.addCleanup(state_path.stop)
        self.real_dispatch = scheduler.dispatch
        self.dispatch = Mock()
        dispatch = patch.object(scheduler, "dispatch", self.dispatch)
        dispatch.start()
        self.addCleanup(dispatch.stop)
        self.real_find_run = scheduler.find_run
        self.find_run = Mock(return_value=None)
        find_run = patch.object(scheduler, "find_run", self.find_run)
        find_run.start()
        self.addCleanup(find_run.stop)
        self.alert = Mock(return_value="issue-url")
        alert = patch.object(scheduler, "alert_missed", self.alert)
        alert.start()
        self.addCleanup(alert.stop)

    def state(self):
        return json.loads(self.path.read_text())

    def test_initialize_starts_next_local_day_to_avoid_transition_duplicate(self):
        scheduler.initialize(utc("2026-09-24T06:00:00Z"))
        self.assertEqual(self.state()["activated_on"], "2026-09-25")
        with self.assertRaises(RuntimeError):
            scheduler.initialize(utc("2026-09-24T06:00:00Z"))

    def test_one_intent_and_dispatch_even_after_ambiguous_response(self):
        scheduler.initialize(utc("2026-09-24T06:00:00Z"))
        def uncertain(day):
            self.assertEqual(self.state()["intents"][day]["status"], "uncertain")
            raise RuntimeError("lost response")
        self.dispatch.side_effect = uncertain
        scheduler.poll(utc("2026-09-25T07:08:00Z"))
        scheduler.poll(utc("2026-09-25T07:09:00Z"))
        self.dispatch.assert_called_once_with("2026-09-25")
        self.alert.assert_not_called()
        scheduler.poll(utc("2026-09-25T07:13:00Z"))
        scheduler.poll(utc("2026-09-25T07:14:00Z"))
        self.alert.assert_called_once_with("2026-09-25")
        self.assertEqual(self.state()["intents"]["2026-09-25"]["alert_url"], "issue-url")

    def test_matching_run_confirms_start_without_another_dispatch(self):
        scheduler.initialize(utc("2026-09-24T06:00:00Z"))
        scheduler.poll(utc("2026-09-25T07:08:00Z"))
        self.find_run.return_value = {"id": 123, "created_at": "2026-09-25T07:09:00Z"}
        scheduler.poll(utc("2026-09-25T07:13:00Z"))
        self.dispatch.assert_called_once_with("2026-09-25")
        self.alert.assert_not_called()
        self.assertEqual(self.state()["intents"]["2026-09-25"]["run_id"], 123)

    def test_late_run_still_alerts_missed_start(self):
        scheduler.initialize(utc("2026-09-24T06:00:00Z"))
        scheduler.poll(utc("2026-09-25T07:08:00Z"))
        self.find_run.return_value = {"id": 123, "created_at": "2026-09-25T07:20:00Z"}
        scheduler.poll(utc("2026-09-25T07:20:00Z"))
        self.alert.assert_called_once_with("2026-09-25")

    def test_machine_missed_full_day_alerts_without_stale_dispatch(self):
        scheduler.initialize(utc("2026-09-24T06:00:00Z"))
        scheduler.poll(utc("2026-09-26T06:00:00Z"))
        self.alert.assert_called_once_with("2026-09-25")
        self.dispatch.assert_not_called()
        self.assertEqual(self.state()["intents"]["2026-09-25"]["status"], "missed")

    def test_prior_day_dispatch_reconciles_before_alerting_after_outage(self):
        for uncertain in (False, True):
            with self.subTest(uncertain=uncertain):
                self.path.unlink(missing_ok=True)
                self.dispatch.reset_mock(side_effect=True)
                self.find_run.reset_mock(return_value=True)
                self.find_run.return_value = None
                self.alert.reset_mock()
                scheduler.initialize(utc("2026-09-24T06:00:00Z"))
                if uncertain:
                    self.dispatch.side_effect = RuntimeError("lost response")
                scheduler.poll(utc("2026-09-25T07:08:00Z"))
                self.find_run.return_value = {"id": 123, "created_at": "2026-09-25T07:09:00Z"}
                scheduler.poll(utc("2026-09-26T06:00:00Z"))
                self.find_run.assert_called_with("2026-09-25", "2026-09-25T07:08:00+00:00")
                self.dispatch.assert_called_once_with("2026-09-25")
                self.alert.assert_not_called()
                self.assertEqual(self.state()["intents"]["2026-09-25"]["run_id"], 123)
                self.assertEqual(self.state()["last_audited_on"], "2026-09-25")

    def test_prior_day_missing_or_late_run_still_alerts(self):
        for run in (None, {"id": 123, "created_at": "2026-09-25T07:20:00Z"}):
            with self.subTest(run=run):
                self.path.unlink(missing_ok=True)
                self.find_run.return_value = None
                self.alert.reset_mock()
                scheduler.initialize(utc("2026-09-24T06:00:00Z"))
                scheduler.poll(utc("2026-09-25T07:08:00Z"))
                self.find_run.return_value = run
                scheduler.poll(utc("2026-09-26T06:00:00Z"))
                scheduler.poll(utc("2026-09-26T06:01:00Z"))
                self.alert.assert_called_once_with("2026-09-25")

    def test_prior_day_lookup_failure_does_not_advance_audit_or_false_alert(self):
        scheduler.initialize(utc("2026-09-24T06:00:00Z"))
        scheduler.poll(utc("2026-09-25T07:08:00Z"))
        self.find_run.side_effect = RuntimeError("GitHub unavailable")
        with self.assertRaisesRegex(RuntimeError, "GitHub unavailable"):
            scheduler.poll(utc("2026-09-26T06:00:00Z"))
        self.assertEqual(self.state()["last_audited_on"], "2026-09-24")
        self.alert.assert_not_called()

    def test_spring_and_fall_chicago_due_times(self):
        self.assertEqual(scheduler.scheduled_at(datetime(2026, 3, 8).date()),
                         utc("2026-03-08T08:00:00Z"))
        self.assertEqual(scheduler.scheduled_at(datetime(2026, 11, 1).date()),
                         utc("2026-11-01T08:08:00Z"))

    def test_find_run_requires_matching_intent_title(self):
        with patch.object(scheduler, "github", return_value={"workflow_runs": [
            {"id": 1, "created_at": "2026-09-25T07:08:02Z", "display_title": "Release"},
            {"id": 2, "created_at": "2026-09-25T07:08:03Z",
             "display_title": "Release / daily 2026-09-25"}]}) as github:
            run = self.real_find_run("2026-09-25", "2026-09-25T07:08:00+00:00")
            self.assertEqual(run["id"], 2)
            github.assert_called_once()

    def test_dispatch_sends_dated_workflow_input(self):
        with patch.object(scheduler.subprocess, "run", return_value=Mock(returncode=0)) as run:
            self.real_dispatch("2026-09-25")
        self.assertEqual(json.loads(run.call_args.kwargs["input"]), {
            "ref": "main", "inputs": {"schedule_intent": "2026-09-25"}})


if __name__ == "__main__":
    unittest.main()
