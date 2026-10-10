# Release supervision

A native T3 task at 02:08 America/Chicago owns dated daily Release dispatch. A
Monday 09:00 task requests image-pin maintenance. A signed T3 Connect webhook
reconciles manual and other main Releases. Tasks return to the existing deployment
orchestration conversation; they do not create a conversation per tick. There is
no recurring minute prompt or daytime polling agent. GitHub cron stays removed,
and pushes or merges to main do not release the application.

Each task invokes `deployment_session.py trigger`. The helper saves the request
before starting `scope-deployment-session.service`. One process holds
`watcher.lock` throughout a bounded session, polls GitHub and durable state every
minute, and exits after independently verified completion, escalation, or its
four-hour deadline plus five minutes to confirm repair termination. Systemd also
caps process lifetime at 250 minutes. A durable budget of three crash recoveries preserves the
saved session deadline and existing repair owner. Exit 75 permits retry only after
that process saved its failed budget; activation/startup failures exit 78 and do
not restart. Import and argument failures also stop. Systemd has no competing
start-rate limit that can block the Python cleanup attempt. A trigger delivered during a
session joins its durable queue. Failed starts retain their request.
An expired session leaves new requests queued while confirming repair termination.
After confirmation it retires its budget, then handles new work in a fresh bounded
session under the same exclusive lock. Unconfirmed termination leaves requests
unhandled and retains repair ownership.

`session.json` records the process ID, session ID, deadline, operation phase,
last completed progress, observed run attempts, and outcome. T3's succeeded task
status proves prompt delivery only. Check the service exit result and saved
progress to establish command execution. Dependency failures retain a bounded
operation name and exception category; raw CLI or provider output is excluded.
Changing the scheduler does not establish the cause of earlier RuntimeError
failures.
Daily reconciliation failures retain their request and continue repair observation.
They publish the sanitized failure without advancing successful progress, defer
maintenance, and end at the existing deadline. A process deadline escapes ordinary
dependency retries and credential-cleanup failures.

## Dispatch and repair safety

The daily intent in `~/.local/state/scope-deployment-watcher/daily-dispatch.json`
is synced before dispatch. An ambiguous API response is reconciled against the
exact dated main Release and never blindly retried. A missing or late start
opens an assigned issue after five minutes. A daily trigger delayed more than
ten minutes does not launch a stale release. Whole missed dates are audited
without catch-up dispatch. Spring's nonexistent 02:08 maps to 03:08, matching
T3's local-time calculation; fall's 02:08 occurs once.

Healthy queued and running releases need deterministic observation only. A
failure or missing final verification starts repair. `supervision.json` retains
one repair conversation and its dedicated worktree. Later releases enter that
owner's inbox. Existing receipts and all worktrees are preserved across crashes,
upgrades, and provider handoffs. A previous agent must be confirmed stopped
before another repair owner starts. Unconfirmed termination retains ownership.

The successful `Verify and record release` job supplies independent completion
proof and exact production deployment receipts. A successful no-change run must
explicitly skip deployment jobs. Corrective receipts map the original run through
an explicit chain to a subsequently verified correction; unrelated successes
cannot close failures. Current attempts are checked independently. At most three
correction run IDs are admitted per investigation, and repair prompts cap
corrective deployments at three. The supervisor permits three agent recoveries,
interrupts twenty-minute inactivity, waits for stop confirmation before fallback,
and escalates unanswered input after ten minutes or a four-hour investigation.
Repair follows protected PR delivery and Scope mirroring. Recurring release-flake
failures must be fixed, not merely rerun.

## Signed release events

`Deployment supervision event` runs code only for main's Release workflow events
(requested, in progress, and completed). GitHub repository hooks cannot filter by
workflow, and T3's native webhook has no payload filter; using a repository-wide
hook would produce unrelated model prompts. The filtered forwarder independently
checks the run through GitHub, signs its exact bounded JSON body with HMAC-SHA256,
and sends it through T3 Connect. Each forwarder run also records the original
Release ID and attempt in its GitHub run title, so the external observer can
find completed retries of old run IDs even when Surface misses their delivery. It follows no redirects and makes no ambiguous
POST retry. The receiver requires `x-hub-signature-256`, hexadecimal encoding,
and the `sha256=` prefix, then independently verifies trusted workflow identity
and current attempt through GitHub again. Run/attempt ownership and deterministic
delivery IDs deduplicate events. Unsigned or invalid signatures are rejected
before any prompt is delivered. T3 Connect expires held deliveries after ten
minutes; the external observer reports missed supervision.

Each run/attempt retains admitted event phases. A new completion phase requeues
an escalated run whose workflow is still open, with its new request time preserved
through expired-session cleanup. Replays of handled phases cannot renew its budget.

Configure repository variable `SCOPE_DEPLOYMENT_WEBHOOK_URL` with the task's
managed URL and Actions secret `SCOPE_DEPLOYMENT_WEBHOOK_SECRET` with the shared
signing secret. Enter that same secret privately through T3's `request_secret`
card and save its one-use reference on the webhook task. Do not put secret values
in chat, files, commands, or task prompts. Keep the webhook paused until signing,
forwarding, and independent deduplication are verified. Without the URL, the
forwarder remains skipped. Required credentials and tunnel access must be ready
before replacing the existing scheduler.

## Weekly image pins

The Monday task creates a durable weekly intent in `image-pin-refresh.json`.
An active release saves a deferral; its bounded session drains that intent after
repair and release ownership end, including across restart or a week boundary.
An escalated investigation remains active for maintenance deferral until GitHub
confirms that its workflow ended.
Ordinary release observation never creates unrequested weekly work. Failed pin
starts reuse the same thread and worktree, retry for ten minutes, then escalate
once. The maintenance agent updates exact pins and checksums, opens a protected
PR only when needed, and never releases the app or weakens image scans.

## While Surface is offline

The external code-only GitHub observer checks expected daily starts, active
session progress, and releases whose event never reached supervision. Idle
sessions need no daytime heartbeat renewal. Scheduling and issue notifications
remain best effort, not a paging SLA. See [heartbeat.md](heartbeat.md).

An assigned outage or missing-start issue gives the maintainer release ownership:

1. Acknowledge ownership in the issue and inspect existing repair work before
   starting another repair.
2. Check recent Release runs and dated dispatch intents. Reconcile an uncertain
   mutation before dispatching manually. Leave `schedule_intent` empty for a
   manual release.
3. Repair through a PR, required checks and reviews, and a corrective release
   with independent production verification. Link the original and correction in
   the issue and preserve the incident receipt chain.

Returning online does not launch a missed dated release. A manual reconcile
trigger observes current runs and existing repair ownership without daily dispatch.

## Install and verified cutover

An operator can quarantine a known GitHub metadata inconsistency that would
otherwise keep supervision active indefinitely. This requires an escalated,
trusted main Release, queued with no jobs and no activity for seven days, and
no repair owner. The command holds the supervision lock, verifies GitHub, and
preserves the run, escalation, receipts and worktrees. It never records a
successful or completed release and does not cancel or delete the GitHub run.

```sh
python3 -B ~/.local/share/scope-automation/bounded-release-supervisor/deployment_session.py quarantine RUN_ID --reason 'Operator disposition and evidence reference' --dry-run
python3 -B ~/.local/share/scope-automation/bounded-release-supervisor/deployment_session.py quarantine RUN_ID --reason 'Operator disposition and evidence reference'
```

The disposition binds to the exact attempt, status, revision, creation/update
and start timestamps. Supervision and the external observer independently check
that fingerprint and the absence of jobs. Any change or new job lifts the
quarantine, resumes observation and retains the original disposition for audit.
Webhook replay must also recheck activity before using a prior acknowledgment.
The observer can alert on new activity even while Surface is idle. Quarantine
is an explicit operator action, never an automatic response to an old queue.
Applying it requires the T3 owner and a stopped bounded session. The command
updates local session state and publishes the disposition and reconciled counts
before returning. If publication fails, repeat the same command and reason to
retry publication without creating another disposition. A lifted disposition
can be renewed after another seven days of inactivity; earlier dispositions
remain in local audit history. Completed runs with lifted dispositions need
no further direct lookup after leaving the recent release listing.

Verify Surface identity (`adam-blumoff-surface-book-2`) and use delivered GitHub
main. Run `./dev/check ops` and `./dev/check guardrails`. Create replacement tasks
paused with stable client request IDs and bindings to the existing conversation.
Preserve `supervision.json`, `daily-dispatch.json`, `image-pin-refresh.json`,
receipts, inboxes, and every repair worktree. Never initialize existing state.

Stage the delivered modules separately so the existing timer continues to use
its installed implementation until cutover:

```sh
install -d -m 700 ~/.local/share/scope-automation/bounded-release-supervisor
install -m 600 deploy/automation/deployment_*.py deploy/automation/image_pin_refresh.py deploy/automation/heartbeat.py ~/.local/share/scope-automation/bounded-release-supervisor/
install -m 644 deploy/automation/scope-deployment-session.service ~/.config/systemd/user/
systemctl --user daemon-reload
```

Finish tunnel and signing setup first. Inspect live release state and actual T3
repair ownership. When no repair is active, disable the old timer and drain its
service. Save `scheduler-owner.json` atomically with owner `t3` and
`activated_on` set to the first dated daily start the new owner must deliver.
For an existing installation, use today's Chicago date when cutting over before
02:08, or the next date after today's start was handled by the previous owner.
For a fresh installation, use the date saved by daily initialization. Preserve
inherited dispatch intents and reconcile any ambiguous dispatch before changing
ownership. This is the only local activation gate.

```sh
systemctl --user disable --now scope-deployment-watcher.timer
systemctl --user is-active scope-deployment-watcher.service
python3 -B ~/.local/share/scope-automation/bounded-release-supervisor/deployment_session.py trigger reconcile
python3 -B ~/.local/share/scope-automation/bounded-release-supervisor/deployment_session.py status
systemctl --user show scope-deployment-session.service -p ActiveState -p Result -p ExecMainStatus
```

Use the forwarder's manual `workflow_dispatch` input `run_id` to send an existing,
already verified main Release as a signing probe; it never starts a Release.
Repeat that probe to establish deduplication and send an unsigned request to
confirm rejection before prompt delivery.

Prove a manual T3 task invocation executes the command and exits, persisted
progress advances, healthy releases create no repair agent, signed delivery and
replay share ownership, invalid signatures deliver no prompt, and the external
observer passes. A task prompt accepted into a busy conversation is not execution
proof. Keep replacements paused until these checks succeed. Then enable exactly
one daily task, one weekly pin task, and the signed event task; verify next run
times in Chicago and confirm the old minute timer is disabled. Retain the old
installed implementation for rollback until verified installation completes.

If cutover fails, pause new tasks and stop/drain the bounded session before
restoring the previous owner. Do not stop an active repair to roll back scheduling.
Before restoring the old implementation, migrate any nonterminal `watching`
run records to its `waiting` state under `watcher.lock`, using atomic `save_json`.
This one-time rollback keeps releases admitted by the new observer visible to
the old repair owner. Preserve all other records, receipts, and worktrees.
Remove `SCOPE_DEPLOYMENT_SESSION_STATUS` when restoring the old timer so the
external observer checks that operating owner's timestamp. The old timer and
its installed modules remain safe while signing or other prerequisites are
missing. After success, record task IDs, schedules, PR/merge and operational
proof in the orchestration conversation, then delete the setup task.

For a first installation, confirm that both `supervision.json` and
`daily-dispatch.json` are absent, then inspect historical releases and active T3
repair ownership. Inspect the read-only preview before initializing:

```sh
python3 -B ~/.local/share/scope-automation/bounded-release-supervisor/deployment_watcher.py --initialize --dry-run
python3 -B ~/.local/share/scope-automation/bounded-release-supervisor/deployment_scheduler.py initialize
python3 -B ~/.local/share/scope-automation/bounded-release-supervisor/deployment_watcher.py --initialize
```

Daily initialization takes the supervision lock, refuses existing state, and
sets dispatch activation to the next Chicago date without dispatching a Release.
Supervision initialization establishes the boundary for completed historical
runs and admits active runs. Neither runs implicitly during a session. Reuse
existing state on an upgrade; do not run these initialization commands. Continue
the verified cutover above, omitting old timer retirement only if none exists.
If first-install preparation was interrupted, inspect the saved JSON and run
only the initialization command for a missing file; preserve existing state.

## Main checks

After the aggregate check has been committed to main, configure protection using
the checked-in payload and enable GitHub auto-merge:

```sh
gh api --method PUT repos/scope-vcs/scope-vcs/branches/main/protection --input deploy/automation/main-protection.json
gh api --method PATCH repos/scope-vcs/scope-vcs -F allow_auto_merge=true
```

This requires the GitHub Actions `Required PR checks` result, including for
administrators, without reviewer approvals, up-to-date branch rebuilds, merge
queues, or post-merge CI. Force pushes and main deletion are disabled. Read the
settings back after applying them. Local tests remain required for explicitly
authorized direct-main changes made before protection is enabled.
