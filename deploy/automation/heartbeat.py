#!/usr/bin/env python3

import argparse
from datetime import date, datetime, timedelta, timezone
import json
import os
import re
import subprocess
import urllib.parse


REPO = "scope-vcs/scope-vcs"
VARIABLE = "SCOPE_DEPLOYMENT_WATCHER_HEARTBEAT"
SESSION_VARIABLE = "SCOPE_DEPLOYMENT_SESSION_STATUS"
REPOSITORY_MAINTAINER = "adamblumoff"
OUTAGE = "<!-- scope-deployment-watch:heartbeat -->"


def gh(*args):
    result = subprocess.run(
        ["gh", *args], capture_output=True, text=True, timeout=45, check=False
    )
    if result.returncode:
        raise RuntimeError("GitHub heartbeat/alert request failed")
    return result.stdout.strip()


def heartbeat(repo=REPO, *, status=None):
    if status is not None:
        value = json.dumps(status, separators=(",", ":"))
        gh("variable", "set", SESSION_VARIABLE, "--repo", repo, "--body", value)
        return value
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
    return gh("issue", "create", "--repo", repo, "--title", title,
              "--body", f"{body}\n\n{marker}", "--assignee", REPOSITORY_MAINTAINER)


def alert(release_id, reason, repo=REPO, *, recoveries=0, thread_id="", provider=""):
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
                 "This issue closes automatically after a healthy check.\n\n"
                 "While this issue is open, no daily release is dispatched and no failed release is "
                 "repaired automatically. The assignee owns both; follow \"While Surface is offline\" in "
                 f"https://github.com/{repo}/blob/main/deploy/automation/OPERATIONS.md.")
    return False


def session_health(value, now, max_age):
    state = json.loads(value)
    if not isinstance(state, dict):
        return False
    if state.get("status") not in {"idle", "running", "escalated"} or state.get("failure"):
        return False
    progress = datetime.fromisoformat(state["progress_at"].replace("Z", "+00:00"))
    if progress.tzinfo is None or (progress - now).total_seconds() > 300:
        return False
    if state["status"] == "running":
        deadline = datetime.fromisoformat(state["deadline_at"])
        return deadline.tzinfo is not None and now <= deadline + timedelta(minutes=5) and (now - progress).total_seconds() <= max_age
    return not state.get("repair_owners") and not state.get("releases")


def release_runs(repo, since):
    from deployment_policy import trusted_run
    runs = {}
    for status in ("", "in_progress", "queued", "requested", "waiting", "pending"):
        page = 1
        while True:
            query = f"branch=main&per_page=100&page={page}" + (f"&status={status}" if status else "")
            batch = json.loads(gh("api", f"repos/{repo}/actions/workflows/release.yml/runs?{query}"))["workflow_runs"]
            for run in batch:
                if trusted_run(run) and (not status or run["status"] == status):
                    runs[str(run["id"])] = run
            if len(batch) < 100 or not status and datetime.fromisoformat(batch[-1]["created_at"].replace("Z", "+00:00")) <= since:
                break
            page += 1
    page = 1
    targets = {}
    while True:
        query = urllib.parse.urlencode({"branch": "main", "per_page": 100, "page": page, "created": ">=" + since.isoformat()})
        batch = json.loads(gh("api", f"repos/{repo}/actions/workflows/deployment-supervision-event.yml/runs?{query}"))["workflow_runs"]
        for event in batch:
            match = re.fullmatch(r"Supervise Release / ([1-9][0-9]*) / attempt ([1-9][0-9]*)", event.get("display_title", ""))
            if (match and event.get("head_branch") == "main" and event.get("event") == "workflow_run"
                    and event.get("path", "").split("@")[0] == ".github/workflows/deployment-supervision-event.yml"
                    and event.get("repository", {}).get("full_name") == repo):
                key = (int(match[1]), int(match[2]))
                targets[key] = min(targets.get(key, event["created_at"]), event["created_at"])
        if len(batch) < 100:
            break
        page += 1
    for (run_id, attempt), seen_at in targets.items():
        run = json.loads(gh("api", f"repos/{repo}/actions/runs/{run_id}"))
        if trusted_run(run) and run["run_attempt"] >= attempt:
            if run["run_attempt"] == attempt:
                run["event_seen_at"] = seen_at
            runs[str(run_id)] = run
    return list(runs.values())


def observe(value, repo=REPO, now=None, max_age=1200):
    from deployment_policy import quarantine_matches, trusted_run
    from deployment_scheduler import START_GRACE, alert_missed, local_date, scheduled_at
    now = now or datetime.now(timezone.utc)
    state = {}
    try:
        state = json.loads(value)
        if not isinstance(state, dict):
            raise ValueError("Session status must be an object")
        installed = datetime.fromisoformat(state["installed_at"].replace("Z", "+00:00"))
        if installed.tzinfo is None:
            raise ValueError("Installation boundary must have a timezone")
        healthy = session_health(value, now, max_age)
    except (KeyError, ValueError, TypeError):
        healthy = False
        state = {}
        installed = now
    day = local_date(now)
    try:
        audited = date.fromisoformat(state.get("daily_audited_on", day.isoformat()))
        activated = date.fromisoformat(state.get("activated_on", day.isoformat()))
    except (ValueError, TypeError):
        audited = activated = day
        healthy = False
    session_ok = healthy
    first = max(activated, min(audited + timedelta(days=1), day))
    since = min(scheduled_at(first), now - timedelta(hours=24))
    current = {str(run["id"]): run for run in release_runs(repo, since)}
    quarantined = state.get("quarantined", {})
    for run_id in quarantined:
        if run_id not in current:
            run = json.loads(gh("api", f"repos/{repo}/actions/runs/{run_id}"))
            if not trusted_run(run):
                raise ValueError("Quarantined workflow is no longer a trusted main Release")
            current[run_id] = run
    runs = []
    quarantine_changed = False
    for run in current.values():
        disposition = quarantined.get(str(run["id"]))
        if disposition:
            if quarantine_matches(run, disposition):
                current_jobs = json.loads(gh("api", f"repos/{repo}/actions/runs/{run['id']}/attempts/{run['run_attempt']}/jobs?per_page=1"))
                if not current_jobs["total_count"]:
                    continue
            quarantine_changed = True
        runs.append(run)
    expected = first
    while expected <= day:
        intended = scheduled_at(expected)
        timely = any(run.get("display_title") == f"Release / daily {expected.isoformat()}"
                     and intended - timedelta(seconds=5) <= datetime.fromisoformat(run["created_at"].replace("Z", "+00:00")) <= intended + START_GRACE
                     for run in runs)
        if now >= intended + START_GRACE and not timely:
            alert_missed(expected.isoformat())
            healthy = False
        expected += timedelta(days=1)
    unobserved = [run for run in runs if
                  (state.get("observed", {}).get(str(run["id"]), {}).get("attempt") != run["run_attempt"] or
                   run["status"] != "completed" and state.get("status") != "running") and
                  (run["status"] != "completed" or datetime.fromisoformat(
                      max(run["created_at"], run.get("run_started_at") or "").replace("Z", "+00:00")) >= installed) and
                  (run["status"] != "completed" or datetime.fromisoformat(
                      max(run["created_at"], run.get("run_started_at") or "", run.get("event_seen_at", "")).replace("Z", "+00:00")) >= since) and
                  now - datetime.fromisoformat(max(run["created_at"], run.get("run_started_at") or "",
                      run.get("event_seen_at", "")).replace("Z", "+00:00")) >= timedelta(minutes=20)]
    if not session_ok or unobserved or quarantine_changed:
        ensure_issue(repo, OUTAGE, "Scope deployment session needs attention",
                     "The T3 deployment session is missing, failed, stalled, has not admitted an expected Release, "
                     "or has not reconciled a changed quarantine. "
                     "Inspect session.json, triggers.json, signed webhook deliveries, T3 task execution, "
                     "and scope-deployment-session.service. Idle daytime is healthy. See "
                     "\"While Surface is offline\" in "
                     f"https://github.com/{repo}/blob/main/deploy/automation/OPERATIONS.md.")
        healthy = False
    else:
        for issue in issues(repo):
            if OUTAGE in issue["body"]:
                gh("issue", "close", str(issue["number"]), "--repo", repo,
                   "--comment", "Expected supervision is healthy; idle time needs no polling heartbeat.")
    return healthy


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Watcher heartbeat and deduplicated GitHub alerts, using the existing gh login.")
    parser.add_argument("command", choices=["check", "publish"])
    parser.add_argument("--repo", default=REPO)
    args = parser.parse_args()
    if args.command == "publish":
        heartbeat(args.repo)
    else:
        value = os.environ.get(SESSION_VARIABLE, "")
        healthy = observe(value, args.repo) if value else check(os.environ.get(VARIABLE, ""), args.repo)
        raise SystemExit(0 if healthy else 1)
