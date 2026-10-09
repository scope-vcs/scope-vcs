from __future__ import annotations

import argparse
from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
import fcntl
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time
import uuid

from deployment_diagnostics import diagnostic, operation
from deployment_policy import DEADLINE_SECONDS, MAX_RECOVERIES, stamp, trusted_run
from deployment_runtime import github, save_json
import deployment_scheduler as scheduler
import deployment_watcher as watcher
import image_pin_refresh as pins
from heartbeat import REPO, ensure_issue, heartbeat

STATE_DIR = watcher.STATE_DIR
SERVICE = "scope-deployment-session.service"
POLL_SECONDS = 60
STOP_GRACE = 300


class SessionDeadlineExceeded(BaseException):
    pass


def read(name, default=None):
    path = STATE_DIR / name
    return json.loads(path.read_text()) if path.exists() else default


@contextmanager
def queue_lock():
    STATE_DIR.mkdir(parents=True, exist_ok=True, mode=0o700)
    with (STATE_DIR / "triggers.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield


def require_owner():
    if read("scheduler-owner.json", {}).get("owner") != "t3":
        raise RuntimeError("T3 cutover has not been verified and activated")


def event_key(payload):
    if (payload.get("event") != "workflow_run" or payload.get("action") not in {"requested", "in_progress", "completed"}
            or payload.get("repository") != "scope-vcs/scope-vcs"):
        return None
    run_id, attempt = payload.get("run_id"), payload.get("attempt")
    if any(not isinstance(value, int) or isinstance(value, bool) or value <= 0 for value in (run_id, attempt)):
        raise ValueError("Webhook must identify a positive run and attempt")
    delivery = payload.get("delivery", "")
    if not isinstance(delivery, str) or not re.fullmatch(r"[a-fA-F0-9-]{36}", delivery):
        raise ValueError("Webhook must identify a GitHub delivery")
    run = github(f"actions/runs/{run_id}")
    if not trusted_run(run) or run["run_attempt"] < attempt:
        return None
    return f"release-{run_id}-{run['run_attempt']}"


def launch():
    with operation("systemd.session-launch"):
        for _ in range(50):
            active = subprocess.run(["systemctl", "--user", "is-active", "--quiet", SERVICE], timeout=10).returncode == 0
            session = read("session.json", {})
            if not active or session.get("status") == "running":
                break
            time.sleep(0.1)
        else:
            raise RuntimeError("Previous session has not exited; trigger remains durable")
        subprocess.run(["systemctl", "--user", "start", "--no-block", SERVICE], check=True, capture_output=True, timeout=10)


def trigger(kind, payload=None, now=None):
    require_owner()
    now = now or datetime.now(timezone.utc)
    payload = payload or {}
    if kind == "webhook":
        with operation("github.webhook-validation"):
            key = event_key(payload)
        if key is None:
            return {"ignored": True}
        event = f"{payload['attempt']}:{payload['action']}"
    elif kind == "daily":
        day = scheduler.local_date(now)
        if now < scheduler.scheduled_at(day):
            return {"ignored": "not-due"}
        key = "daily-" + day.isoformat()
    elif kind == "pins":
        week, due = pins.current_week(now)
        if now < due:
            return {"ignored": "not-due"}
        key = "pins-" + week
    elif kind == "reconcile":
        key = "reconcile-" + str(uuid.uuid4())
    else:
        raise ValueError("Unknown session trigger")
    with queue_lock():
        queue = read("triggers.json", {"requests": {}})
        existing = queue["requests"].get(key)
        if existing is None:
            queue["requests"][key] = {"kind": kind, "revision": 1, "requested_at": now.isoformat()}
            if kind == "webhook":
                queue["requests"][key]["events"] = [event]
            save_json(STATE_DIR / "triggers.json", queue)
        elif "handled_at" in existing and kind == "webhook":
            run_id = key.split("-")[1]
            record = read("supervision.json", {"runs": {}})["runs"].get(run_id, {})
            if (record and not watcher.release_open(record)
                    and record.get("attempt") == int(key.split("-")[2])):
                return {"deduplicated": key}
        if existing is not None and kind == "webhook":
            events = existing.setdefault("events", [])
            if event not in events:
                existing.update(events=events + [event], revision=existing["revision"] + 1,
                                requested_at=now.isoformat())
                existing.pop("handled_at", None)
                save_json(STATE_DIR / "triggers.json", queue)
            elif "handled_at" in existing:
                return {"deduplicated": key}
        launch()
    return {"queued": key, "service": SERVICE}


def publish(session):
    session["installed_at"] = read("supervision.json")["installed_at"]
    save_json(STATE_DIR / "session.json", session)
    with operation("github.session-heartbeat"):
        heartbeat(status={key: value for key, value in session.items() if key != "admitted_requests"})


def pending_requests():
    with queue_lock():
        queue = read("triggers.json", {"requests": {}})
        return {key: value for key, value in queue["requests"].items() if "handled_at" not in value}


def admit_requests(session):
    requests = pending_requests()
    session["admitted_requests"].update({key: value["revision"] for key, value in requests.items()})
    save_json(STATE_DIR / "session.json", session)
    return requests


def retire_requests(session, reason):
    requests = pending_requests()
    admitted = session.get("admitted_requests", {})
    abandoned = {key: revision for key, revision in admitted.items()
                 if key in requests and requests[key]["revision"] == revision}
    if abandoned:
        with operation("github.trigger-escalation"):
            url = ensure_issue(REPO, f"<!-- scope-deployment-watch:session:{session['id']} -->",
                               "Deployment session stopped with unhandled triggers",
                               f"Bounded session {session['id']} stopped with {len(abandoned)} unhandled requests. "
                               "Automatic handling has ended. Inspect session.json, triggers.json, daily-dispatch.json "
                               "and deployment receipts on the supervision host. Reconcile any ambiguous dispatch "
                               "before taking manual action; preserved repair worktrees remain available.", state="all")
        with queue_lock():
            queue = read("triggers.json")
            for key, revision in abandoned.items():
                request = queue["requests"][key]
                request.setdefault("retirements", {})[session["id"]] = {
                    "revision": revision, "at": stamp(), "reason": reason, "alert_url": url}
                if request["revision"] == revision:
                    request["handled_at"] = stamp()
            save_json(STATE_DIR / "triggers.json", queue)
    with queue_lock():
        queue = read("triggers.json", {"requests": {}})
        return sum(session["id"] in value.get("retirements", {}) for value in queue["requests"].values())


@contextmanager
def process_deadline(deadline):
    def exceeded(signum, frame):
        raise SessionDeadlineExceeded("Bounded supervision process deadline exceeded")
    previous = signal.signal(signal.SIGALRM, exceeded)
    remaining = max(0.01, (deadline - datetime.now(timezone.utc)).total_seconds())
    signal.setitimer(signal.ITIMER_REAL, remaining)
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


def stop_previous(previous, reason, deadline=None):
    deadline = deadline or datetime.now(timezone.utc) + timedelta(seconds=STOP_GRACE)
    previous.update(status="running", phase="stopping", stop_reason=reason,
                    stop_deadline_at=deadline.isoformat(), pid=os.getpid())
    save_json(STATE_DIR / "session.json", previous)
    try:
        with process_deadline(deadline):
            while True:
                if datetime.now(timezone.utc) >= deadline:
                    raise SessionDeadlineExceeded("Repair termination was not confirmed before the stop deadline")
                with operation("t3.stop-confirmation"):
                    owners = watcher.stop_owned(reason)
                releases = sum(watcher.release_open(value) for value in read("supervision.json")["runs"].values())
                previous.update(repair_owners=owners, releases=releases, progress_at=stamp())
                save_json(STATE_DIR / "session.json", previous)
                if not owners:
                    previous["escalated_requests"] = retire_requests(previous, reason)
                    previous.update(status="escalated", phase="stopped", ended_at=stamp())
                    if previous["escalated_requests"]:
                        previous.setdefault("failure", {"operation": "session.trigger-retirement", "category": "UnhandledTriggers"})
                    else:
                        previous.pop("failure", None)
                        previous.pop("failed_at", None)
                    break
                time.sleep(POLL_SECONDS)
    except (Exception, SessionDeadlineExceeded) as error:
        previous.update(status="escalated", failed_at=stamp(), failure=diagnostic(error))
        raise
    finally:
        publish(previous)


def run(now=None):
    require_owner()
    with (STATE_DIR / "watcher.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError("Another supervision owner is still running") from None
        while True:
            result = run_session(now)
            if result.get("phase") != "stopped" or not pending_requests():
                return result
            now = None


def run_session(now=None):
    at = now or datetime.now(timezone.utc)
    previous = read("session.json", {})
    supervision = read("supervision.json", {"threads": {}})
    continuing = previous.get("status") in {"running", "failed"} or (
        previous.get("status") == "escalated" and previous.get("phase") != "stopped") or (
        bool(previous) and any(value.get("owns_agent") for value in supervision["threads"].values()))
    recoveries = previous.get("recoveries", 0) + 1 if continuing else 0
    obsolete = continuing and at >= datetime.fromisoformat(previous["deadline_at"])
    exhausted = recoveries > MAX_RECOVERIES
    if obsolete or exhausted:
        try:
            stop_previous(previous, "attempts_exhausted" if exhausted else "deadline_exceeded")
        except (Exception, SessionDeadlineExceeded) as error:
            previous.update(status="escalated", failed_at=stamp(), failure=diagnostic(error))
            save_json(STATE_DIR / "session.json", previous)
            return previous
        return previous
    session = {"id": previous["id"] if continuing else str(uuid.uuid4()),
               "started_at": previous["started_at"] if continuing else at.isoformat(),
               "deadline_at": previous["deadline_at"] if continuing else (at + timedelta(seconds=DEADLINE_SECONDS)).isoformat(),
               "status": "running", "pid": os.getpid(), "phase": "starting",
               "recoveries": recoveries,
               "admitted_requests": previous.get("admitted_requests", {}) if continuing else {},
               "activated_on": read("scheduler-owner.json")["activated_on"],
               "progress_at": previous.get("progress_at", at.isoformat()) if continuing else at.isoformat()}
    deadline = datetime.fromisoformat(session["deadline_at"])
    try:
        with process_deadline(deadline + timedelta(seconds=STOP_GRACE)):
            admit_requests(session)
            publish(session)
            return supervise_session(session, deadline)
    except (Exception, SessionDeadlineExceeded) as error:
        failure = diagnostic(error)
        session.update(status="escalated" if failure["category"] == "SessionDeadlineExceeded" else "failed",
                       failed_at=stamp(), failure=failure)
        save_json(STATE_DIR / "session.json", session)
        if session["status"] == "escalated":
            return session
        raise


def supervise_session(session, deadline):
    while True:
        at = datetime.now(timezone.utc)
        expired = at >= deadline
        if expired:
            stop_previous(session, "deadline_exceeded", deadline + timedelta(seconds=STOP_GRACE))
            return session
        requests = admit_requests(session)
        session["phase"] = "daily-reconciliation"
        save_json(STATE_DIR / "session.json", session)
        days = [key.removeprefix("daily-") for key in requests if key.startswith("daily-")]
        daily = None
        daily_failure = None
        try:
            with operation("github.daily-dispatch"):
                daily = scheduler.poll(at, dispatch_day=scheduler.local_date(at).isoformat() if
                                       scheduler.local_date(at).isoformat() in days else None)
        except Exception as error:
            daily_failure = diagnostic(error)
        session["phase"] = "release-supervision"
        save_json(STATE_DIR / "session.json", session)
        run_ids = [int(key.split("-")[1]) for key in requests if key.startswith("release-")]
        result = watcher.poll(run_ids=run_ids)
        supervision = read("supervision.json")
        release_open = bool(result["active_releases"] or result["repair_owners"])
        with operation("t3.image-pin-refresh"):
            pending_pins = pins.poll(release_open=release_open or daily is None, now=at,
                                     requested_weeks=tuple(key.removeprefix("pins-") for key in requests if key.startswith("pins-")))
        intent = daily["intents"].get(scheduler.local_date(at).isoformat(), {}) if daily is not None else {}
        uncertain = daily is None or ("dispatched_at" in intent and "run_id" not in intent
                     and at < datetime.fromisoformat(intent["scheduled_at"]) + scheduler.START_GRACE)
        session.update(phase="observing", releases=result["active_releases"],
                       repair_owners=result["repair_owners"],
                       observed={key: {"attempt": value["attempt"], "status": value["status"]}
                             for key, value in supervision["runs"].items()
                             if datetime.fromisoformat(max(value["created_at"], value.get("attempt_started_at", "")).replace("Z", "+00:00")) >= at - timedelta(days=2)})
        if daily_failure:
            session.update(phase="daily-reconciliation-failed", failure=daily_failure, failed_at=stamp())
        else:
            session.update(progress_at=stamp(), daily_audited_on=daily["last_audited_on"])
            session.pop("failure", None)
            session.pop("failed_at", None)
        with queue_lock():
            queue = read("triggers.json", {"requests": {}})
            for key, request in requests.items():
                if queue["requests"][key]["revision"] == request["revision"] and (daily is not None or not key.startswith("daily-")):
                    queue["requests"][key]["handled_at"] = stamp()
            save_json(STATE_DIR / "triggers.json", queue)
            new_work = any("handled_at" not in value for value in queue["requests"].values())
            if not release_open and not uncertain and not pending_pins and not new_work:
                session.update(status="idle", phase="completed", ended_at=stamp())
            publish(session)
            if session["status"] != "running":
                return session
        time.sleep(POLL_SECONDS)


def main():
    parser = argparse.ArgumentParser(description="Durable triggers and bounded Scope release supervision")
    subcommands = parser.add_subparsers(dest="command", required=True)
    subcommands.add_parser("run")
    trigger_parser = subcommands.add_parser("trigger")
    trigger_parser.add_argument("kind", choices=["daily", "webhook", "pins", "reconcile"])
    subcommands.add_parser("status")
    args = parser.parse_args()
    if args.command == "trigger":
        payload = json.loads(sys.stdin.read(8193)) if args.kind == "webhook" else None
        result = trigger(args.kind, payload)
    elif args.command == "run":
        result = run()
    else:
        result = {"owner": read("scheduler-owner.json", {}), "session": read("session.json", {}),
                  "triggers": read("triggers.json", {})}
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    try:
        main()
    except (Exception, SessionDeadlineExceeded) as error:
        print(json.dumps({"failed": diagnostic(error)}), flush=True)
        try:
            saved = read("session.json", {})
            retryable = len(sys.argv) > 1 and sys.argv[1] == "run" and saved.get("pid") == os.getpid() and saved.get("status") == "failed"
        except Exception:
            retryable = False
        raise SystemExit(75 if retryable else 78)
