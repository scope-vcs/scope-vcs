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


if __name__ == "__main__":
    unittest.main()
