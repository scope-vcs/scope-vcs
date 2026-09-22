"""Release completion and agent recovery rules, independent of T3 and GitHub I/O."""
from __future__ import annotations

from datetime import datetime, timezone

MAX_RECOVERIES = 3
IDLE_SECONDS = 20 * 60
INPUT_SECONDS = 10 * 60
RETRY_SECONDS = 2 * 60
DEADLINE_SECONDS = 4 * 60 * 60
TERMINAL = {"verified", "no_change", "recovered", "escalated"}


def timestamp(value: str) -> float:
    return datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()


def stamp() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def trusted_run(run: dict) -> bool:
    return (run.get("head_branch") == "main"
            and run.get("event") in {"schedule", "workflow_dispatch"}
            and run.get("repository", {}).get("full_name") == "scope-vcs/scope-vcs"
            and run.get("path", "").split("@")[0] == ".github/workflows/release.yml")


def completion(run: dict, jobs: list[dict]) -> str | None:
    if run.get("status") != "completed" or run.get("conclusion") != "success":
        return None
    by_name = {job["name"]: job.get("conclusion") for job in jobs}
    if by_name.get("Verify and record release") == "success":
        return "verified"
    # A successful run with no selected deployable components has no live revision to verify.
    # Require explicit skipped jobs so missing/incomplete API data never counts as success.
    if (by_name.get("Plan selected components") == "success"
            and by_name.get("Validate selected components / Production validation gate") == "success"
            and all(by_name.get(name) == "skipped" for name in (
                "Verify and record release", "Backend deploy", "Web deploy", "CLI deploy"))):
        return "no_change"
    return None


def running(thread: dict) -> bool:
    return ((thread.get("latestTurn") or {}).get("state") == "running"
            or (thread.get("session") or {}).get("status") in {"starting", "running"})


def last_activity(thread: dict, fallback: str) -> float:
    turn = thread.get("latestTurn") or {}
    dates = [fallback, turn.get("requestedAt"), turn.get("startedAt"), turn.get("completedAt")]
    dates += [item.get("updatedAt") or item.get("createdAt") for item in thread.get("messages", [])]
    dates += [item.get("createdAt") for item in thread.get("activities", [])]
    return max(timestamp(value) for value in dates if value)


def supervise(info: dict, thread: dict, now: str) -> tuple[str, str]:
    """Decide wait/resume/fallback/interrupt/escalate; the caller persists effects."""
    at = timestamp(now)
    if thread.get("archivedAt") or thread.get("deletedAt"):
        return "escalate", "agent_unavailable"
    deadline_exceeded = at - timestamp(info["created_at"]) >= DEADLINE_SECONDS
    if info.get("stopping_at"):
        if running(thread):
            return ("escalate", "agent_unavailable") if at - timestamp(info["stopping_at"]) >= 300 else ("wait", "")
        if info.get("stop_reason") == "deadline_exceeded" or deadline_exceeded:
            return "escalate", "deadline_exceeded"
        if info["recoveries"] >= MAX_RECOVERIES:
            return "escalate", "attempts_exhausted"
        return "fallback" if info["provider"] == "codex" else "resume", "agent_unavailable"
    if deadline_exceeded:
        return ("interrupt" if running(thread) else "escalate"), "deadline_exceeded"
    idle = at - last_activity(thread, info["dispatch_at"])
    if thread.get("hasPendingApprovals") or thread.get("hasPendingUserInput"):
        return ("escalate", "approval_required") if idle >= INPUT_SECONDS else ("wait", "")
    if running(thread):
        return ("interrupt", "agent_unavailable") if idle >= IDLE_SECONDS else ("wait", "")
    if at - timestamp(info["dispatch_at"]) < RETRY_SECONDS:
        return "wait", ""
    if info["recoveries"] >= MAX_RECOVERIES:
        return "escalate", "attempts_exhausted"
    error = (thread.get("latestTurn") or {}).get("state") == "error"
    return ("fallback" if info["provider"] == "codex" and (error or info["recoveries"] >= 1) else "resume"), "agent_unavailable"
