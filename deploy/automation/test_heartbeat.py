import json
from datetime import datetime, timedelta, timezone
import unittest
from unittest.mock import patch

import heartbeat


class HeartbeatTests(unittest.TestCase):
    def setUp(self):
        self.now = datetime(2026, 9, 22, 12, 0, tzinfo=timezone.utc)

    def test_poll_records_time(self):
        with patch.object(heartbeat, "gh", return_value="") as gh:
            value = heartbeat.heartbeat()
        self.assertIsNotNone(datetime.fromisoformat(value).tzinfo)
        self.assertEqual(gh.call_args.args, (
            "variable", "set", heartbeat.VARIABLE, "--repo", heartbeat.REPO, "--body", value))

    def test_stale_poll_creates_assigned_issue(self):
        with patch.object(heartbeat, "gh", side_effect=["[]", "issue-url"]) as gh:
            self.assertFalse(heartbeat.check(
                (self.now - timedelta(minutes=21)).isoformat(), now=self.now))
        self.assertIn("--assignee", gh.call_args.args)
        self.assertIn("adamblumoff", gh.call_args.args)
        body = gh.call_args.args[gh.call_args.args.index("--body") + 1]
        self.assertIn('"While Surface is offline"', body)

    def test_missing_invalid_and_future_heartbeat_are_unhealthy(self):
        values = ["", "no date", self.now.replace(tzinfo=None).isoformat(),
                  (self.now + timedelta(minutes=6)).isoformat()]
        for value in values:
            with self.subTest(value=value), patch.object(heartbeat, "ensure_issue"):
                self.assertFalse(heartbeat.check(value, now=self.now))

    def test_outage_is_deduplicated(self):
        issue = {"number": 1, "body": heartbeat.OUTAGE, "url": "issue-url"}
        with patch.object(heartbeat, "gh", return_value=json.dumps([issue])) as gh:
            self.assertFalse(heartbeat.check("", now=self.now))
        self.assertEqual(gh.call_count, 1)

    def test_recovery_closes_only_heartbeat_outage(self):
        issues = [{"number": 1, "body": heartbeat.OUTAGE, "url": "issue-url"},
                  {"number": 2, "body": "Some other issue", "url": "other-url"}]
        with patch.object(heartbeat, "gh", side_effect=[json.dumps(issues), ""]) as gh:
            self.assertTrue(heartbeat.check(self.now.isoformat(), now=self.now))
        self.assertEqual(gh.call_count, 2)
        self.assertEqual(gh.call_args.args[:3], ("issue", "close", "1"))

    def test_release_alert_is_deduplicated_even_when_closed(self):
        issue = {"number": 1, "body": "<!-- scope-deployment-watch:release:123 -->", "url": "issue-url"}
        with patch.object(heartbeat, "gh", return_value=json.dumps([issue])) as gh:
            self.assertEqual(heartbeat.alert(123, "attempts_exhausted"), "issue-url")
        self.assertEqual(gh.call_count, 1)
        self.assertIn("all", gh.call_args.args)

    def test_provider_error_never_appears_in_issue(self):
        with patch.object(heartbeat, "gh", side_effect=["[]", "issue-url"]) as gh:
            heartbeat.alert(123, "secret provider exception")
        self.assertNotIn("secret provider exception", " ".join(gh.call_args.args))

    def test_publish_failure_propagates_for_retry(self):
        with patch.object(heartbeat, "gh", side_effect=RuntimeError("Failed")):
            with self.assertRaises(RuntimeError):
                heartbeat.heartbeat()

    def test_alert_includes_bounded_recovery_context(self):
        thread_id = "12345678-1234-1234-1234-123456789abc"
        with patch.object(heartbeat, "gh", side_effect=["[]", "issue-url"]) as gh:
            heartbeat.alert(123, "attempts_exhausted", recoveries=3,
                            thread_id=thread_id, provider="claudeAgent")
        body = gh.call_args.args[gh.call_args.args.index("--body") + 1]
        self.assertIn("3 agent recoveries", body)
        self.assertIn(thread_id, body)
        self.assertIn("claudeAgent", body)

    def test_alert_rejects_uncontrolled_context(self):
        for context in [{"recoveries": "provider secret"}, {"thread_id": "provider secret"},
                        {"provider": "provider secret"}]:
            with self.subTest(context=context), patch.object(heartbeat, "gh") as gh:
                with self.assertRaises(ValueError):
                    heartbeat.alert(123, "attempts_exhausted", **context)
            gh.assert_not_called()

    def test_invalid_release_id_cannot_be_published(self):
        with patch.object(heartbeat, "gh") as gh:
            with self.assertRaises(ValueError):
                heartbeat.alert("bad input", "attempts_exhausted")
        gh.assert_not_called()


class SessionObserverTests(unittest.TestCase):
    def setUp(self):
        self.now = datetime(2026, 10, 8, 20, tzinfo=timezone.utc)
        self.state = {"status": "idle", "progress_at": "2026-10-08T09:00:00Z", "repair_owners": 0,
                      "releases": 0, "activated_on": "2026-10-08", "daily_audited_on": "2026-10-07",
                      "observed": {"123": {"attempt": 1, "status": "verified"}}}
        self.run = {"id": 123, "run_attempt": 1, "status": "completed", "conclusion": "success",
                    "created_at": "2026-10-08T07:08:04Z", "display_title": "Release / daily 2026-10-08",
                    "head_branch": "main", "event": "workflow_dispatch", "path": ".github/workflows/release.yml",
                    "repository": {"full_name": "scope-vcs/scope-vcs"}}

    def observe(self, runs=None):
        with patch.object(heartbeat, "issues", return_value=[]), \
                patch.object(heartbeat, "gh", return_value=json.dumps({"workflow_runs": runs if runs is not None else [self.run]})):
            return heartbeat.observe(json.dumps(self.state), now=self.now)

    def test_idle_daytime_needs_no_fresh_heartbeat_but_active_sessions_do(self):
        with patch.object(heartbeat, "ensure_issue") as issue:
            self.assertTrue(self.observe())
            issue.assert_not_called()
            self.state.update(status="running", deadline_at="2026-10-08T23:00:00Z", releases=1)
            self.assertFalse(self.observe())
            issue.assert_called_once()

    def test_missing_daily_start_is_independent_of_a_healthy_idle_session(self):
        with patch("deployment_scheduler.alert_missed", return_value="issue-url") as missed:
            self.assertFalse(self.observe([]))
        missed.assert_called_once_with("2026-10-08")

    def test_manual_failure_and_new_attempt_without_observation_alert(self):
        for status, conclusion in (("in_progress", None), ("completed", "failure"), ("completed", "success")):
            with self.subTest(status=status, conclusion=conclusion), patch.object(heartbeat, "ensure_issue") as issue:
                run = self.run | {"id": 456, "status": status, "conclusion": conclusion, "display_title": "Release",
                                  "created_at": "2026-10-08T19:00:00Z", "run_attempt": 2}
                self.state["observed"]["456"] = {"attempt": 1, "status": "verified"}
                self.assertFalse(self.observe([self.run, run]))
                issue.assert_called_once()
        self.state["observed"]["456"] = {"attempt": 2, "status": "escalated"}
        with patch.object(heartbeat, "ensure_issue") as issue:
            self.assertTrue(self.observe([self.run, run]))
            issue.assert_not_called()

    def test_expired_session_and_unconfirmed_repair_owner_remain_unhealthy(self):
        self.state.update(status="running", progress_at=self.now.isoformat(), deadline_at="2026-10-08T19:54:00Z")
        with patch.object(heartbeat, "ensure_issue"):
            self.assertFalse(self.observe())
            self.state.update(status="escalated", repair_owners=1)
            self.assertFalse(self.observe())

    def test_historical_failures_are_not_event_gaps_but_an_old_run_active_retry_is(self):
        historical = self.run | {"id": 456, "created_at": "2026-09-01T07:08:00Z", "conclusion": "failure", "display_title": "Release"}
        with patch.object(heartbeat, "ensure_issue") as issue:
            self.assertTrue(self.observe([self.run, historical]))
            issue.assert_not_called()
            retry = historical | {"status": "in_progress", "run_attempt": 2, "run_started_at": "2026-10-08T19:00:00Z"}
            self.assertFalse(self.observe([self.run, retry]))
            issue.assert_called_once()

    def test_completed_old_run_retry_is_found_through_durable_github_event_receipts(self):
        event = {"id": 987, "display_title": "Supervise Release / 456 / attempt 2", "created_at": "2026-10-08T19:00:00Z",
                 "head_branch": "main", "event": "workflow_run", "path": ".github/workflows/deployment-supervision-event.yml",
                 "repository": {"full_name": heartbeat.REPO}}
        retry = self.run | {"id": 456, "run_attempt": 2, "created_at": "2026-08-01T07:08:00Z",
                           "run_started_at": "2026-10-08T19:00:00Z", "conclusion": "failure", "display_title": "Release"}
        def request(*args):
            if "deployment-supervision-event.yml/runs?" in args[1]:
                return json.dumps({"workflow_runs": [event]})
            if args[1].endswith("actions/runs/456"):
                return json.dumps(retry)
            return json.dumps({"workflow_runs": [self.run]})
        with patch.object(heartbeat, "gh", side_effect=request), patch.object(heartbeat, "issues", return_value=[]), \
                patch.object(heartbeat, "ensure_issue") as issue:
            self.assertFalse(heartbeat.observe(json.dumps(self.state), now=self.now))
            issue.assert_called_once()
            self.state["observed"]["456"] = {"attempt": 2, "status": "escalated"}
            self.assertTrue(heartbeat.observe(json.dumps(self.state), now=self.now))

    def test_old_nonterminal_retries_are_found_when_event_delivery_is_missing(self):
        for status in ("in_progress", "queued", "requested", "waiting", "pending"):
            retry = self.run | {"id": 456, "run_attempt": 2, "status": status, "created_at": "2026-08-01T07:08:00Z",
                               "run_started_at": "2026-10-08T19:00:00Z", "display_title": "Release"}
            def request(*args):
                if f"&status={status}" in args[1]:
                    return json.dumps({"workflow_runs": [retry]})
                return json.dumps({"workflow_runs": [self.run]})
            with self.subTest(status=status), patch.object(heartbeat, "gh", side_effect=request), \
                    patch.object(heartbeat, "issues", return_value=[]), patch.object(heartbeat, "ensure_issue") as issue:
                self.assertFalse(heartbeat.observe(json.dumps(self.state), now=self.now))
                issue.assert_called_once()


if __name__ == "__main__":
    unittest.main()
