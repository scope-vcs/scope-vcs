from datetime import datetime
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, Mock, patch

import image_pin_refresh as refresh


def utc(value):
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


# 2026-10-05 is a Monday; 09:00 in Chicago is 14:00 UTC.
BEFORE_DUE = utc("2026-10-05T13:59:00Z")
DUE = utc("2026-10-05T14:00:00Z")


class ImagePinRefreshTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = Path(directory.name) / "image-pin-refresh.json"
        self.start = Mock()
        self.alert = Mock(return_value="issue-url")
        for name, value in {"STATE_PATH": self.path, "start": self.start, "alert_not_started": self.alert}.items():
            patcher = patch.object(refresh, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def weeks(self):
        return json.loads(self.path.read_text())["weeks"]

    def test_starts_one_agent_per_week_from_monday_morning_in_chicago(self):
        refresh.poll(False, BEFORE_DUE)
        self.start.assert_not_called()
        refresh.poll(False, DUE)
        refresh.poll(False, utc("2026-10-08T02:00:00Z"))
        self.start.assert_called_once()
        week, intent = self.start.call_args.args
        self.assertEqual(week, "2026-W41")
        self.assertEqual(self.weeks()["2026-W41"]["started_at"], "2026-10-05T14:00:00Z")
        refresh.poll(False, utc("2026-10-12T14:00:00Z"))
        self.assertEqual(self.start.call_count, 2)
        self.assertNotEqual(self.start.call_args.args[1]["thread_id"], intent["thread_id"])

    def test_waits_for_an_open_release_investigation_including_before_a_retry(self):
        refresh.poll(True, DUE)
        self.start.assert_not_called()
        self.assertFalse(self.path.exists())
        self.start.side_effect = RuntimeError("T3 unavailable")
        refresh.poll(False, utc("2026-10-05T16:30:00Z"))
        refresh.poll(True, utc("2026-10-05T16:31:00Z"))
        self.start.assert_called_once()
        refresh.poll(False, utc("2026-10-05T16:32:00Z"))
        self.assertEqual(self.start.call_count, 2)

    def test_failed_start_retries_the_same_thread_and_reports_once_after_the_grace_period(self):
        self.start.side_effect = RuntimeError("T3 unavailable")
        refresh.poll(False, DUE)
        refresh.poll(False, utc("2026-10-05T14:09:00Z"))
        self.alert.assert_not_called()
        self.alert.side_effect = [RuntimeError("GitHub unavailable"), "issue-url"]
        refresh.poll(False, utc("2026-10-05T14:10:00Z"))
        refresh.poll(False, utc("2026-10-05T14:11:00Z"))
        refresh.poll(False, utc("2026-10-05T14:11:30Z"))
        self.assertEqual(self.alert.call_count, 2)
        self.assertEqual(self.weeks()["2026-W41"]["alert_url"], "issue-url")
        self.start.side_effect = None
        refresh.poll(False, utc("2026-10-05T14:12:00Z"))
        refresh.poll(False, utc("2026-10-05T14:13:00Z"))
        self.assertEqual(self.start.call_count, 6)
        self.assertEqual(len({call.args[1]["thread_id"] for call in self.start.call_args_list}), 1)
        self.assertEqual(self.weeks()["2026-W41"]["started_at"], "2026-10-05T14:12:00Z")


class StartTests(unittest.TestCase):
    def test_prepares_the_worktree_then_creates_the_thread_and_starts_its_turn(self):
        events = []
        client = MagicMock()
        client.dispatch.side_effect = lambda command: events.append(command["type"])
        t3 = MagicMock()
        t3.return_value.__enter__.return_value = client
        intent = {"created_at": "2026-10-05T14:00:00Z", "thread_id": "thread", "worktree": "/worktrees/pins"}
        with patch.object(refresh, "T3Client", t3), \
                patch.object(refresh, "create_worktree", side_effect=lambda path: events.append(str(path))):
            refresh.start("2026-W41", intent)
        self.assertEqual(events, ["/worktrees/pins", "thread.create", "message.dispatch"])
        create, turn = (call.args[0] for call in client.dispatch.call_args_list)
        self.assertEqual((create["threadId"], create["worktreePath"]), ("thread", "/worktrees/pins"))
        self.assertEqual((turn["threadId"], turn["text"]), ("thread", refresh.PROMPT))
        self.assertEqual(turn["modelSelection"]["instanceId"], refresh.PRIMARY_PROVIDER)


if __name__ == "__main__":
    unittest.main()
