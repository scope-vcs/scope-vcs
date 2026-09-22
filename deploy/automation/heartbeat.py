#!/usr/bin/env python3
"""Watcher heartbeat and deduplicated GitHub alerts, using the existing gh login."""

import argparse
from datetime import datetime, timezone
import json
import os
import re
import subprocess


REPO = "scope-vcs/scope-vcs"
VARIABLE = "SCOPE_DEPLOYMENT_WATCHER_HEARTBEAT"
ASSIGNEE = "adamblumoff"
OUTAGE = "<!-- scope-deployment-watch:heartbeat -->"


def gh(*args):
    result = subprocess.run(
        ["gh", *args], capture_output=True, text=True, timeout=45, check=False
    )
    if result.returncode:
        # Provider output can contain credentials or private prompt content.
        raise RuntimeError("GitHub heartbeat/alert request failed")
    return result.stdout.strip()


def heartbeat(repo=REPO):
    """Call only after an entire watcher poll, including supervision, succeeds."""
    now = datetime.now(timezone.utc).isoformat()
    gh("variable", "set", VARIABLE, "--repo", repo, "--body", now)
    return now


def issues(repo, state="open"):
    return json.loads(gh("issue", "list", "--repo", repo, "--state", state,
                         "--limit", "1000", "--json", "number,body,url"))


def ensure_issue(repo, marker, title, body, state="open"):
    existing = next((issue for issue in issues(repo, state) if marker in issue["body"]), None)
    if existing:
        return existing["url"]
    # Keep the confirmed repository maintainer explicit. Workflow tokens identify
    # github-actions[bot], which cannot receive assigned-issue notifications.
    return gh("issue", "create", "--repo", repo, "--title", title,
              "--body", f"{body}\n\n{marker}", "--assignee", ASSIGNEE)


def alert(release_id, reason, repo=REPO, *, recoveries=0, thread_id="", provider=""):
    """Escalate once per release; raw agent/provider errors are never published."""
    release_id = str(release_id)
    if not re.fullmatch(r"[0-9]+", release_id):
        raise ValueError("Release ID must be numeric")
    if not isinstance(recoveries, int) or isinstance(recoveries, bool) or recoveries < 0:
        raise ValueError("Recovery count must be a nonnegative integer")
    if thread_id and not re.fullmatch(r"[a-fA-F0-9-]{36}", thread_id):
        raise ValueError("Expected a T3 thread UUID")
    if provider not in {"", "codex", "claudeAgent"}:
        raise ValueError("Unexpected deployment agent provider")
    marker = f"<!-- scope-deployment-watch:release:{release_id} -->"
    # The watcher supplies a controlled reason code, not a provider exception.
    reasons = {
        "attempts_exhausted": "The deployment agent exhausted its recovery attempts.",
        "deadline_exceeded": "The deployment exceeded its recovery time limit.",
        "agent_unavailable": "The deployment agent could not be started or resumed.",
        "approval_required": "The deployment agent needs an approval to continue.",
        "verification_failed": "The expected production revision could not be verified.",
    }
    explanation = reasons.get(reason, "The deployment watcher could not finish recovery automatically.")
    body = (f"{explanation}\n\nRelease: https://github.com/{repo}/actions/runs/{release_id}\n\n"
            f"The supervisor attempted {recoveries} agent recoveries after the initial dispatch.\n\n"
            + (f"Last provider: `{provider}`.\n\n" if provider else "")
            + (f"T3 conversation ID: `{thread_id}` on Surface.\n\n" if thread_id else "")
            + "Inspect the Surface deployment watcher state and its T3 conversation for the "
            "attempts and blocker. Automatic recovery is paused for this release.")
    return ensure_issue(repo, marker, f"Deployment recovery needs attention: {release_id}", body, "all")


def check(value, repo=REPO, max_age=1200, now=None):
    """External observer: raise an assigned issue once and close it on recovery."""
    now = now or datetime.now(timezone.utc)
    try:
        timestamp = datetime.fromisoformat(value)
        if timestamp.tzinfo is None:
            raise ValueError("Heartbeat must have a timezone")
        age = (now - timestamp).total_seconds()
        healthy = -300 <= age <= max_age
    except (ValueError, TypeError):
        healthy = False
    if healthy:
        for issue in issues(repo):
            if OUTAGE in issue["body"]:
                gh("issue", "close", str(issue["number"]), "--repo", repo,
                   "--comment", "The external monitor received a recent successful watcher poll.")
        return True
    ensure_issue(repo, OUTAGE, "Surface deployment watcher stopped reporting",
                 "No recent successful deployment watcher poll was recorded. "
                 "Surface may be offline, the watcher may be failing, or its GitHub access may have expired.\n\n"
                 "Check scope-deployment-watcher.timer and scope-deployment-watcher.service on Surface. "
                 "The watcher normally reports every minute. This external check considers it overdue "
                 f"after {max_age // 60} minutes. GitHub may delay scheduled checks. "
                 "This issue closes automatically after a healthy check.")
    return False


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["check", "publish"])
    parser.add_argument("--repo", default=REPO)
    args = parser.parse_args()
    if args.command == "publish":
        heartbeat(args.repo)
    else:
        raise SystemExit(0 if check(os.environ.get(VARIABLE, ""), args.repo) else 1)
