#!/usr/bin/env python3
import importlib.util
import pathlib
import unittest

SPEC = importlib.util.spec_from_file_location(
    "check_workflow_timeouts", pathlib.Path(__file__).with_name("check-workflow-timeouts.py")
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class MissingTimeoutsTest(unittest.TestCase):
    def test_reports_jobs_without_a_positive_integer_timeout(self):
        workflows = {
            "a.yml": "jobs:\n  ok:\n    runs-on: x\n    timeout-minutes: 5\n  none:\n    runs-on: x\n",
            "b.yml": "jobs:\n  zero:\n    runs-on: x\n    timeout-minutes: 0\n  text:\n    runs-on: x\n    timeout-minutes: '5'\n",
        }
        self.assertEqual(
            MODULE.missing_timeouts(workflows),
            ["a.yml: none", "b.yml: zero", "b.yml: text"],
        )

    def test_reusable_workflow_calls_are_exempt(self):
        workflows = {"c.yml": "jobs:\n  call:\n    uses: ./.github/workflows/other.yml\n"}
        self.assertEqual(MODULE.missing_timeouts(workflows), [])

    def test_repository_workflows_all_declare_timeouts(self):
        workflows = {str(p): p.read_text() for p in MODULE.WORKFLOWS.glob("*.yml")}
        self.assertGreater(len(workflows), 0)
        self.assertEqual(MODULE.missing_timeouts(workflows), [])


if __name__ == "__main__":
    unittest.main()
