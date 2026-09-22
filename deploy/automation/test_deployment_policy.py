"""Release supervision must retain unresolved work until verified or escalated."""
import unittest
from datetime import datetime, timedelta, timezone

import deployment_policy as policy


BASE = datetime(2026, 9, 22, tzinfo=timezone.utc)


def at(seconds):
    return (BASE + timedelta(seconds=seconds)).isoformat().replace("+00:00", "Z")


def info(**changes):
    return {"created_at": at(0), "dispatch_at": at(0), "provider": "codex", "recoveries": 0, **changes}


def thread(state="completed", **changes):
    return {"latestTurn": {"state": state, "requestedAt": at(0)}, **changes}


def run(**changes):
    return {"status": "completed", "conclusion": "success", "head_branch": "main",
            "event": "schedule", "repository": {"full_name": "scope-vcs/scope-vcs"},
            "path": ".github/workflows/release.yml", **changes}


def job(name, conclusion):
    return {"name": name, "conclusion": conclusion}


class CompletionTests(unittest.TestCase):
    def test_only_main_release_workflows_are_trusted(self):
        self.assertTrue(policy.trusted_run(run()))
        self.assertTrue(policy.trusted_run(run(event="workflow_dispatch")))
        for changes in ({"head_branch": "feature"}, {"event": "pull_request"},
                        {"repository": {"full_name": "someone/fork"}},
                        {"path": ".github/workflows/ci.yml"}, {"repository": {}}):
            with self.subTest(changes=changes):
                self.assertFalse(policy.trusted_run(run(**changes)))

    def test_early_release_failures_and_incomplete_runs_remain_open(self):
        jobs = [job("Verify and record release", "success")]
        for state in ("queued", "in_progress", "waiting"):
            self.assertIsNone(policy.completion(run(status=state), jobs))
        for conclusion in ("failure", "cancelled", "timed_out", "neutral", None):
            self.assertIsNone(policy.completion(run(conclusion=conclusion), jobs))
        self.assertIsNone(policy.completion(run(conclusion="failure"), [job("Plan selected components", "failure")]))

    def test_deployment_requires_successful_final_verification(self):
        self.assertEqual(policy.completion(run(), [job("Verify and record release", "success")]), "verified")
        for conclusion in ("failure", "cancelled", "skipped", None):
            self.assertIsNone(policy.completion(run(), [job("Verify and record release", conclusion)]))
        self.assertIsNone(policy.completion(run(), []))

    def test_no_change_requires_complete_explicit_skips_and_successful_validation(self):
        jobs = [job("Plan selected components", "success"),
                job("Validate selected components / Production validation gate", "success")]
        jobs += [job(name, "skipped") for name in (
            "Verify and record release", "Backend deploy", "Web deploy", "CLI deploy")]
        self.assertEqual(policy.completion(run(), jobs), "no_change")
        for index in range(len(jobs)):
            with self.subTest(missing=jobs[index]["name"]):
                self.assertIsNone(policy.completion(run(), jobs[:index] + jobs[index + 1:]))
        for index in range(2):
            failed = [dict(item) for item in jobs]
            failed[index]["conclusion"] = "failure"
            self.assertIsNone(policy.completion(run(), failed))
        deployed = [dict(item) for item in jobs]
        deployed[-1]["conclusion"] = "success"
        self.assertIsNone(policy.completion(run(), deployed))


class SupervisionTests(unittest.TestCase):
    def test_fresh_active_turn_waits_and_recent_activity_resets_idle_timer(self):
        self.assertEqual(policy.supervise(info(), thread("running"), at(600)), ("wait", ""))
        for changes in ({"messages": [{"createdAt": at(1190)}]},
                        {"messages": [{"createdAt": at(0), "updatedAt": at(1190)}]},
                        {"activities": [{"createdAt": at(1190)}]}):
            self.assertEqual(policy.supervise(info(), thread("running", **changes), at(1250)), ("wait", ""))

    def test_session_starting_is_active_even_before_turn_arrives(self):
        for state in ("starting", "running"):
            self.assertTrue(policy.running({"session": {"status": state}}))
        self.assertFalse(policy.running({"session": None, "latestTurn": None}))

    def test_finished_agent_is_resumed_after_grace_period_not_marked_done(self):
        self.assertEqual(policy.supervise(info(), thread(), at(119)), ("wait", ""))
        self.assertEqual(policy.supervise(info(), thread(), at(120)), ("resume", "agent_unavailable"))
        self.assertEqual(policy.supervise(info(recoveries=1), thread(), at(120)), ("fallback", "agent_unavailable"))

    def test_errors_fallback_and_fallback_provider_errors_resume(self):
        self.assertEqual(policy.supervise(info(), thread("error"), at(120)), ("fallback", "agent_unavailable"))
        self.assertEqual(policy.supervise(info(provider="claudeAgent"), thread("error"), at(120)), ("resume", "agent_unavailable"))

    def test_recovery_attempts_are_bounded_for_both_providers(self):
        for provider in ("codex", "claudeAgent"):
            self.assertEqual(policy.supervise(info(provider=provider, recoveries=policy.MAX_RECOVERIES), thread(), at(120)),
                             ("escalate", "attempts_exhausted"))

    def test_stalled_agent_is_interrupted_before_handoff(self):
        self.assertEqual(policy.supervise(info(), thread("running"), at(policy.IDLE_SECONDS)), ("interrupt", "agent_unavailable"))
        stopped = info(stopping_at=at(policy.IDLE_SECONDS), stop_reason="agent_unavailable")
        self.assertEqual(policy.supervise(stopped, thread("running"), at(policy.IDLE_SECONDS + 60)), ("wait", ""))
        self.assertEqual(policy.supervise(stopped, thread(), at(policy.IDLE_SECONDS + 60)), ("fallback", "agent_unavailable"))
        self.assertEqual(policy.supervise(stopped, thread("running"), at(policy.IDLE_SECONDS + 300)), ("escalate", "agent_unavailable"))

    def test_stopped_handoffs_cannot_bypass_attempt_limit(self):
        stopped = info(stopping_at=at(1200), stop_reason="agent_unavailable", recoveries=policy.MAX_RECOVERIES)
        for provider in ("codex", "claudeAgent"):
            self.assertEqual(policy.supervise({**stopped, "provider": provider}, thread(), at(1260)),
                             ("escalate", "attempts_exhausted"))

    def test_pending_input_and_approval_wait_then_escalate(self):
        for flag in ("hasPendingApprovals", "hasPendingUserInput"):
            self.assertEqual(policy.supervise(info(), thread("running", **{flag: True}), at(599)), ("wait", ""))
            self.assertEqual(policy.supervise(info(), thread("running", **{flag: True}), at(600)), ("escalate", "approval_required"))

    def test_deadline_interrupts_active_agent_and_escalates_idle_agent(self):
        self.assertEqual(policy.supervise(info(), thread("running"), at(policy.DEADLINE_SECONDS)), ("interrupt", "deadline_exceeded"))
        self.assertEqual(policy.supervise(info(), thread(), at(policy.DEADLINE_SECONDS)), ("escalate", "deadline_exceeded"))

    def test_deadline_interrupt_that_never_finishes_is_escalated(self):
        stopped = info(stopping_at=at(policy.DEADLINE_SECONDS), stop_reason="deadline_exceeded")
        self.assertEqual(policy.supervise(stopped, thread("running"), at(policy.DEADLINE_SECONDS + 60)), ("wait", ""))
        action, _ = policy.supervise(stopped, thread("running"), at(policy.DEADLINE_SECONDS + 300))
        self.assertEqual(action, "escalate")
        self.assertEqual(policy.supervise(stopped, thread(), at(policy.DEADLINE_SECONDS + 60)), ("escalate", "deadline_exceeded"))

    def test_removed_threads_escalate(self):
        for flag in ("archivedAt", "deletedAt"):
            self.assertEqual(policy.supervise(info(), thread(**{flag: at(60)}), at(120)), ("escalate", "agent_unavailable"))


if __name__ == "__main__":
    unittest.main()
