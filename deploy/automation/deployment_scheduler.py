"""Durable Chicago-date ownership for the daily Release workflow dispatch."""
from __future__ import annotations

from datetime import date, datetime, timedelta, timezone
import json
from pathlib import Path
import subprocess
from zoneinfo import ZoneInfo

from deployment_runtime import REPOSITORY, github, save_json
from heartbeat import ensure_issue

ZONE = ZoneInfo("America/Chicago")
HOUR = 2
MINUTE = 8
START_GRACE = timedelta(minutes=5)
STATE_PATH = Path.home() / ".local/state/scope-deployment-watcher/daily-dispatch.json"


def local_date(now: datetime) -> date:
    return now.astimezone(ZONE).date()


def scheduled_at(day: date) -> datetime:
    # A nonexistent spring-forward wall time resolves to the first real local
    # minute afterward. The fall-back 02:08 wall time occurs only once.
    wall = datetime(day.year, day.month, day.day, HOUR, MINUTE, tzinfo=ZONE)
    instant = wall.astimezone(timezone.utc)
    if instant.astimezone(ZONE).hour != HOUR:
        return instant.astimezone(ZONE).replace(hour=3, minute=0).astimezone(timezone.utc)
    return instant


def initialize(now: datetime | None = None) -> dict:
    if STATE_PATH.exists():
        raise RuntimeError("Daily dispatch state already exists")
    now = now or datetime.now(timezone.utc)
    state = {"activated_on": (local_date(now) + timedelta(days=1)).isoformat(),
             "last_audited_on": local_date(now).isoformat(), "intents": {}}
    save_json(STATE_PATH, state)
    return state


def dispatch(day: str) -> None:
    try:
        result = subprocess.run(
            ["gh", "api", "--method", "POST",
             f"repos/{REPOSITORY}/actions/workflows/release.yml/dispatches",
             "--input", "-"], input=json.dumps({"ref": "main", "inputs": {
                 "schedule_intent": day}}), capture_output=True, text=True, timeout=45)
    except (OSError, subprocess.SubprocessError):
        raise RuntimeError("Daily release dispatch response was uncertain") from None
    if result.returncode:
        # A failed client response is ambiguous: GitHub may have accepted the
        # request. The persisted intent must not be dispatched again blindly.
        raise RuntimeError("Daily release dispatch response was uncertain")


def find_run(day: str, dispatched_at: str) -> dict | None:
    title = f"Release / daily {day}"
    cutoff = datetime.fromisoformat(dispatched_at) - timedelta(seconds=5)
    page = 1
    while True:
        batch = github(
            "actions/workflows/release.yml/runs?branch=main&event=workflow_dispatch"
            f"&per_page=100&page={page}")["workflow_runs"]
        for run in batch:
            if (run.get("display_title") == title
                    and datetime.fromisoformat(run["created_at"].replace("Z", "+00:00")) >= cutoff):
                return run
        if (len(batch) < 100 or datetime.fromisoformat(
                batch[-1]["created_at"].replace("Z", "+00:00")) < cutoff):
            return None
        page += 1


def alert_missed(day: str) -> str:
    marker = f"<!-- scope-deployment-watch:daily-dispatch:{day} -->"
    return ensure_issue(
        REPOSITORY, marker, f"Daily release did not start on {day}",
        f"The {day} Chicago daily Release workflow had no matching run within "
        "five minutes of its intended start. Inspect the Surface deployment watcher, "
        "its daily-dispatch.json intent, GitHub Actions, and the release workflow. "
        "An uncertain dispatch response is never retried automatically.", "all")


def reconcile_start(day: str, intent: dict, now: datetime) -> None:
    if "run_id" not in intent and "dispatched_at" in intent:
        run = find_run(day, intent["dispatched_at"])
        if run is not None:
            intent.update(run_id=run["id"], run_created_at=run["created_at"], status="started")
    deadline = datetime.fromisoformat(intent["scheduled_at"]) + START_GRACE
    if now >= deadline and "alert_url" not in intent:
        if ("run_id" not in intent or datetime.fromisoformat(
                intent["run_created_at"].replace("Z", "+00:00")) > deadline):
            intent["alert_url"] = alert_missed(day)


def poll(now: datetime | None = None) -> dict:
    now = now or datetime.now(timezone.utc)
    if now.tzinfo is None:
        raise ValueError("Scheduler time must include a timezone")
    if not STATE_PATH.exists():
        raise RuntimeError("Daily dispatch state missing; initialize during the scheduler cutover")
    state = json.loads(STATE_PATH.read_text())
    today = local_date(now)
    activated = date.fromisoformat(state["activated_on"])
    if today < activated:
        return state
    previous = date.fromisoformat(state["last_audited_on"]) + timedelta(days=1)
    while previous < today:
        missed = state["intents"].setdefault(previous.isoformat(), {
            "scheduled_at": scheduled_at(previous).isoformat(), "status": "missed"})
        reconcile_start(previous.isoformat(), missed, now)
        state["last_audited_on"] = previous.isoformat()
        save_json(STATE_PATH, state)
        previous += timedelta(days=1)
    day = today.isoformat()
    due = scheduled_at(today)
    if now < due:
        return state
    intent = state["intents"].get(day)
    if intent is None:
        intent = {"scheduled_at": due.isoformat(), "dispatched_at": now.isoformat(),
                  "status": "uncertain"}
        state["intents"][day] = intent
        save_json(STATE_PATH, state)
        try:
            dispatch(day)
        except RuntimeError:
            # Keep watching for an accepted run and alert at the deadline.
            pass
        else:
            intent["status"] = "accepted"
            save_json(STATE_PATH, state)
    reconcile_start(day, intent, now)
    save_json(STATE_PATH, state)
    return state
