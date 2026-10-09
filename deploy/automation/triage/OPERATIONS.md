# Telemetry triage drafts

This helper performs read-only source collection and manages local investigation drafts. It does not create, update, reopen, or close GitHub issues. It does not dispatch repairs, releases, or schedules. GitHub release supervision retains its existing owner.

Run from the repository root with Python 3.11 or later:

```sh
python3 -B -m deploy.automation.triage.session sweep
python3 -B -m deploy.automation.triage.session status
python3 -B -m deploy.automation.triage.session drafts
```

The SQLite ledger defaults to `$XDG_STATE_HOME/scope-triage/ledger.sqlite3`, or `~/.local/state/scope-triage/ledger.sqlite3`. `--state PATH` selects a different ledger outside this checkout. Protect its parent directory and backups as operator state. Preserve this file when changing checkouts or installing a new helper. Do not delete it to recover a stalled investigation.

## Collection contracts

Each source has an independent cursor. A sweep starts with the previous cursor minus five minutes, or the last hour on first use. `--since` and `--until` select the initial range; `--since` cannot reset an existing cursor. The helper commits observations and cursor advancement together only after a complete adapter result. Degraded reads retain the cursor. Exit 1 means one or more sources are degraded; exit 2 means the command or receipt was invalid. A completed source read does not establish service health or an error rate.

Railway requires the installed CLI's environment-scoped `trace status`, `trace list`, and `trace get` commands. The production environment and service inventory come from `.github/deployment-services.json`. An explicit `--environment-id` must match production or staging in that manifest; their incidents remain separate. The helper verifies enabled tracing and at least one received service span for each application service. It hashes operation names, discards span events and messages, and retains only trace, deployment, release and failure identity. Health routes are excluded. A full result cap causes window subdivision; a full single-second window, span cap, or request budget leaves the source incomplete. Railway clamps old queries to retention: historical completeness beyond the provider's retention cannot be established by this helper. Review retention before choosing an old initial range.

Browser input requires `POSTHOG_PERSONAL_API_KEY`, numeric `POSTHOG_PROJECT_ID`, and optionally `POSTHOG_APP_HOST` set to the US or EU PostHog app host. The capture token is insufficient. Queries select only the diagnostic event ID, time, kind, origin, route category and release. Route names are hashed before persistence; personal identities and event payloads are never selected. Grouping remains provisional. Browser events do not contain trace IDs, exception text or stack traces.

GitHub uses the authenticated `gh` CLI for `scope-vcs/scope-vcs`. Open bug reports require `bug` or `release-flake` labels. Pull requests and arbitrary issue text are excluded. A reported bug is a report, not a verified failure. `release-flake` reports and Release workflow failures remain release-owned and cannot be investigated by triage.

`github-runs-created` is a bounded historical backfill using GitHub's creation index. It enumerates attempts of discovered runs and records the attempt's update time as evidence. Its cursor records run creation time. A pending run, 1,000-result search cap, or exhausted request budget prevents advancement. **This source cannot discover an old run retried after its creation window has passed.** Do not use it as proof of continuous failed-attempt coverage. The later signed event trigger must independently retrieve the exact run and attempt before live activation. Failed jobs with the same workflow and conclusion currently form a provisional group; source review is required to establish whether they share a cause.

All remote reads share a per-source 40-request, 120-second budget, with individual calls limited to 30 seconds. Sources exceeding their budget remain degraded. The helper makes no automatic retry loop. Status retains the failed source, last complete cursor, active claims, deadlines and draft counts. The operator or later trigger owns backoff and alerts.

## Bounded T3 investigations

Use one stable T3 orchestration thread. T3 owns agent execution; this helper owns incident identity, dispatch intents, capacity and result acceptance. Do not create separate top-level threads for investigators.

1. Call `orchestrator_capabilities` and save its structured result to a private JSON file outside the checkout.
2. Run `prepare --capabilities PATH --provider PROVIDER --model MODEL --options '{"reasoningEffort":"medium"}'` using an available provider/model and valid options from that catalog.
3. Execute each returned `delegate_task`, `task_status`, or `task_cancel` request through T3. Immediately save the complete structured tool receipt to a private file and run `receive --fingerprint FINGERPRINT --generation GENERATION --receipt PATH`. A dispatch intent precedes the tool call; its request ID, packet, target and deadline survive restart.
4. For completion notifications, call `task_status` and feed that receipt through `receive`. The result summary must be the exact JSON contract in the dispatch prompt. Invalid results leave the claim held for inspection. A terminal task with pending child work cannot release capacity.
5. Revisit `prepare` before each claim's deadline. At expiry it emits cancellation for the persisted task ID. Cancellation acceptance alone is insufficient: call `task_status` until terminal with no pending child runs, then pass that receipt to `receive`. A late result cannot become a draft.
6. Run `drafts` to render the durable draft outbox for policy review. No publisher is installed.

Run `prepare` without a capabilities file to recover polling and cancellation when a provider or model becomes unavailable. It reports new work as blocked while retaining controls for saved task IDs. If later evidence transfers an incident to release supervision, its saved draft is withdrawn from the review outbox while its evidence stays in the ledger.

There are at most two active claims, each with a fifteen-minute result deadline. Expiry fences result acceptance but does not itself stop a remote process: the orchestrator must execute cancellation and confirm termination. If it disappears, the ledger keeps capacity occupied. Every previously emitted dispatch without a returned task ID remains blocked for manual reconciliation with T3, even before expiry. Only newly reserved claims emit a dispatch request. T3 scopes request deduplication to its provider session, so replay after restart could create another child. Never replay an ambiguous request or generate a new request ID to bypass it. Recover its actual T3 task ID and bind its receipt before proceeding. Finished or stopped incidents are not automatically investigated again in this draft phase.

Investigators receive sanitized observations as untrusted data and run in approval-required mode. Their schema separates referenced observations from provisional hypothesis, confidence, severity, impact, owner and next action. Free-form output is rejected, as are references absent from the packet. Reproduction remains unknown because this phase supplies no validated reproduction receipt. A draft is a classification and evidence request, not a confirmed causal diagnosis. Review raw details separately within the source's authorized interface when needed.

## Activation gate and current evidence

On 2026-10-08, installed Railway CLI 5.64.1 reported all ten production services with tracing disabled and no span timestamps. The repository deployment CLI pin remains 5.60.0. A capped API log sample contained no trace IDs. No live trace/log correlation was established. Browser capture configuration was present, but analytics query credentials were absent from the implementation shell. These gaps must be repaired and rechecked before depending on telemetry coverage; no service settings were changed by this implementation.

Deployment migration PR #596 was open at implementation start. It owns replacement of the old timer with bounded T3 sessions. Triage does not change its scheduler, task definitions, dispatch intents, receipts or worktrees.

Before activation, review representative drafts and decide publication authority, cadence, provider/model and budget. Subsequent work must implement signed event intake with exact run-attempt verification, serialized issue publication and ambiguous-write reconciliation, an independent progress observer, and post-release evidence checks. Triggers and automatic issue writes stay disabled until that policy review. Closing an issue will require an actually deployed fix and exercised traffic; silence is not resolution.

## Verification

`./dev/check ops` discovers the `test_triage_*.py` suites. They exercise adapter transport contracts, complete-range cursor commits, overlapping reads, SQLite restarts and competing claims, release ownership, single-emission dispatch intents, task receipts, late results, and draft privacy. `./dev/check guardrails` checks repository policy. Live telemetry coverage and authorized activation remain separate acceptance checks.
