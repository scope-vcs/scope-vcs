import sys
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from drill import assess, check_start


CAPTURED = datetime(2026, 9, 27, 7, 17, tzinfo=timezone.utc)
STARTED = CAPTURED + timedelta(hours=3)
PASSED = {"git": "passed: clones and push", "private_read": "passed", "private_hidden": "passed", "media": "passed: checksum"}


class DrillVerdictTests(unittest.TestCase):
    def test_complete_only_when_every_canary_passes_within_both_targets(self):
        self.assertTrue(assess(CAPTURED, STARTED, STARTED + timedelta(hours=4), PASSED, None)["complete"])
        cases = {
            "failed canary": (STARTED + timedelta(hours=1), {**PASSED, "media": "failed: no attachment"}, None),
            "no canaries": (STARTED + timedelta(hours=1), {}, None),
            "stack failure": (STARTED + timedelta(hours=1), PASSED, "docker run failed"),
            "slow recovery": (STARTED + timedelta(hours=4, minutes=1), PASSED, None),
        }
        for name, (finished, canaries, failure) in cases.items():
            with self.subTest(name):
                self.assertFalse(assess(CAPTURED, STARTED, finished, canaries, failure)["complete"])

    def test_stale_backup_is_incomplete(self):
        started = CAPTURED + timedelta(hours=26, minutes=1)
        verdict = assess(CAPTURED, started, started + timedelta(hours=1), PASSED, None)
        self.assertFalse(verdict["complete"])
        self.assertEqual(verdict["backup_age_hours"], 26.02)

    def test_start_must_fall_between_capture_and_now(self):
        now = STARTED + timedelta(minutes=5)
        check_start(CAPTURED, STARTED, now)
        for started in (CAPTURED - timedelta(seconds=1), now + timedelta(seconds=1), STARTED.replace(tzinfo=None)):
            with self.subTest(started=started), self.assertRaises(ValueError):
                check_start(CAPTURED, started, now)


if __name__ == "__main__":
    unittest.main()
