#!/usr/bin/env python3
"""Require an explicit timeout on every GitHub Actions job.

GitHub's default is 360 minutes, so a hung Postgres or Playwright process
burns six hours of a paid runner. Jobs that call a reusable workflow cannot
declare a timeout; their called jobs carry their own.
"""

import pathlib
import sys

import yaml

WORKFLOWS = pathlib.Path(".github/workflows")


def missing_timeouts(workflows):
    """Return "path: job" for each job that runs steps without timeout-minutes."""
    missing = []
    for path, text in sorted(workflows.items()):
        document = yaml.safe_load(text) or {}
        for name, job in (document.get("jobs") or {}).items():
            if "uses" in job:
                continue
            minutes = job.get("timeout-minutes")
            if not isinstance(minutes, int) or isinstance(minutes, bool) or minutes < 1:
                missing.append(f"{path}: {name}")
    return missing


def main():
    workflows = {str(path): path.read_text() for path in sorted(WORKFLOWS.glob("*.yml"))}
    missing = missing_timeouts(workflows)
    if missing:
        print("Workflow jobs without timeout-minutes:", file=sys.stderr)
        for entry in missing:
            print(f"- {entry}", file=sys.stderr)
        return 1
    print(f"Workflow timeouts: every job in {len(workflows)} workflows declares timeout-minutes.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
