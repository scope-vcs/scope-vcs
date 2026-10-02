# Release supervision

Surface runs `scope-deployment-watcher.timer` every minute, all day. The watcher
dispatches the daily Release workflow at 2:08 a.m. America/Chicago and starts a
T3 agent as soon as a main Release workflow appears, including queued validation
and failures before staging. PR checks remain the only merge-time CI. Release
retains manual dispatch. GitHub cron must be removed before installing this
scheduler so only Surface owns automatic release dispatch.

The daily intent lives in `~/.local/state/scope-deployment-watcher/daily-dispatch.json`.
It is synced to disk before the GitHub API call, using the intended Chicago date. The
workflow records that date in its run title. A lost or failed API response is
ambiguous and is never retried automatically; the watcher looks for the exact
dated run on subsequent polls. An assigned GitHub issue reports a missing or late
run after five minutes. If Surface is offline for a whole date, its next poll
alerts on the missed date without launching a stale release. Start-deadline issues
remain open for investigation even if a run later appears; finding a run proves
dispatch, not recovery from the scheduling delay or successful deployment. The external
heartbeat alerts while Surface cannot poll. On the spring daylight-saving change,
the nonexistent 2:08 a.m. runs at 3:00 a.m.; the fall 2:08 a.m. occurs once.
If a scheduler lookup fails, the same invocation still supervises existing
releases but withholds its heartbeat; the external observer then reports the
broken dispatch owner.
The Scope mirror also runs checks on requests and manual starts, not pushes to main.

The watcher tracks releases in
`~/.local/state/scope-deployment-watcher/supervision.json`. Successful dispatch
means monitoring, not completion. GitHub's successful `Verify and record release`
job supplies the independent production evidence. A successful no-change run must
explicitly skip all deployment jobs. Corrective runs require an agent-written
mapping in the incident's `receipts` file and their own successful production
verification. A failed correction may link to another correction; the whole
chain resolves to its final independently verified run. Duplicate polls preserve
the same result. Unrelated successful runs do not silently resolve older failures.

One conversation owns an open investigation. Newly discovered releases appear in
its `inboxes` file, which the prompt requires the agent to read each monitoring
cycle. Repairs use PRs with squash auto-merge after `Required PR checks` passes.
They never bypass protection or push directly to main. An interrupted worktree is
preserved for the replacement provider.

The supervisor allows three agent recoveries after the initial dispatch. It
resumes a prematurely finished agent after two minutes, falls back from Claude to
Codex on an error or repeated early exit, and interrupts an agent with no
activity for twenty minutes. It waits for the old agent to stop before resuming.
Unanswered approval/input requests escalate after ten minutes. An investigation
has a four-hour recovery limit. The agent is instructed to cap corrective
deployments at three. An assigned GitHub issue records exhausted recovery or a
reported blocker. Escalation stops automatic repair; an agent that cannot be
stopped retains ownership, preventing overlapping repairs.

See [heartbeat.md](heartbeat.md) for the external missing-heartbeat alert. It uses
the existing GitHub login and sends no build jobs. GitHub scheduling and issue
notification preferences affect alert delivery; this is not a paging SLA.

## Weekly image pin refresh

The same watcher poll starts one T3 agent each week, from Monday 9:00 a.m.
America/Chicago, to refresh the pinned packages in the checks and media worker
images before a stale pin fails the release scan. The agent opens a PR only when a
pin changes and enables squash auto-merge after `Required PR checks` passes; that
CI builds and scans each image whose files changed. The scan gate is unchanged,
and a failed release scan is still repaired by the release agent.

The weekly intent lives in `~/.local/state/scope-deployment-watcher/image-pin-refresh.json`
and is written before dispatch, so a retry reuses the same thread. The refresh
waits while a release investigation is open. If Surface is offline on Monday, it
starts at the next poll in that week; a missed week is skipped. The watcher does
not supervise this agent: a start that fails for ten minutes opens an assigned
issue, and an agent that stops early is not resumed. The first poll after
installation starts the current week's refresh if Monday 9:00 a.m. has passed.

## While Surface is offline

Automatic dispatch and repair run only on Surface; nothing takes over while it is
offline. An open "Surface deployment watcher stopped reporting" issue hands both to
its assignee, the on-call maintainer (currently `adamblumoff`), until the issue
closes:

1. Comment on the issue to acknowledge that you own releases until it closes.
2. After 2:08 a.m. Chicago, check `gh run list --workflow release.yml --limit 5`.
   If no release ran that date, dispatch one with
   `gh workflow run release.yml --ref main`. Leave `schedule_intent` empty; that
   input belongs to the watcher.
3. Repair a failed release with a PR, following the same rules as the agent:
   squash auto-merge after `Required PR checks`, then a manual release that passes
   its own production verification. Link the failed and corrective runs in the
   issue.

When Surface returns on the same Chicago date, it dispatches that date's daily run
if it has not already; with nothing new on main, that run skips every deployment
job. For whole dates it missed, it opens "Daily release did not start" issues;
close them with a link to your manual run. It also opens an investigation for any
failed release it had not seen. Its agent should find your corrective run from the
issue links and record it in the incident receipt; the release resolves only
through that receipt, as described above.

## Install or upgrade on Surface

Verify hostname `adam-blumoff-surface-book-2` and fleet identity `surface` before
changing files. Run the tests from the repository root:

```sh
python3 -B -m unittest discover -s deploy/automation -p 'test_*.py'
```

Deploy the Release workflow change first. Confirm main has no `schedule` event,
has the `schedule_intent` workflow-dispatch input and dated `run-name`, and retains
manual dispatch and release concurrency. Do not install the scheduler while the
GitHub cron is still active. Stop the watcher timer and wait for its current
service invocation to finish. Inspect any existing T3 deployment monitor before
initializing new supervision state. Do not start a second agent while an old one
is repairing a release. Upgrade only when no investigation is open: an open
incident keeps its stored provider, and a version that changes the primary
provider would treat that provider with the new roles.

From a checkout of the delivered main revision, install the Python modules and
systemd units. Preserve `supervision.json`, receipts, inboxes, and any existing
agent worktree:

```sh
systemctl --user stop scope-deployment-watcher.timer
while systemctl --user is-active --quiet scope-deployment-watcher.service; do sleep 2; done
install -d -m 700 ~/.local/share/scope-automation/release-supervisor ~/.local/state/scope-deployment-watcher
install -m 600 deploy/automation/deployment_watcher.py deploy/automation/deployment_policy.py deploy/automation/deployment_runtime.py deploy/automation/deployment_scheduler.py deploy/automation/image_pin_refresh.py deploy/automation/heartbeat.py ~/.local/share/scope-automation/release-supervisor/
install -d -m 755 ~/.config/systemd/user
install -m 644 deploy/automation/scope-deployment-watcher.service deploy/automation/scope-deployment-watcher.timer ~/.config/systemd/user/
systemctl --user daemon-reload
if test ! -e ~/.local/state/scope-deployment-watcher/daily-dispatch.json; then
  python3 -B ~/.local/share/scope-automation/release-supervisor/deployment_watcher.py --initialize-scheduler
fi
```

The one-time scheduler initialization begins automatic dispatch on the next
Chicago calendar date, avoiding a duplicate release on the cutover date. Merge
the workflow change and install the scheduler on the same Chicago date, after
that date's 2:08 a.m. release has run. With cron removed, any date between the
merge and the activation date has no automatic release and is not reported as
missed. If
`daily-dispatch.json` already exists, inspect it and skip `--initialize-scheduler`;
the command deliberately refuses to overwrite prior dispatch history. A missing
or corrupt scheduler state after installation fails the poll, which eventually
raises the external heartbeat alert. Verify the `gh` login on Surface can call
`workflow_dispatch`, list workflow runs, and create assigned issues. The watcher
does not need a second timer or another local credential.

For a first install with no `supervision.json`, run these commands before enabling
the timer:

```sh
python3 -B ~/.local/share/scope-automation/release-supervisor/deployment_watcher.py --initialize --dry-run
python3 -B ~/.local/share/scope-automation/release-supervisor/deployment_watcher.py --initialize
```

Supervisor initialization establishes a boundary for completed historical runs,
so deployment failures already repaired before installation are not reactivated.
It still adopts unfinished releases. Subsequent invocations never initialize
state implicitly. The previous watcher's `state.json` is historical operational
evidence and is not read by this supervisor.

Then enable the existing minute timer and inspect its next invocation:

```sh
systemctl --user enable --now scope-deployment-watcher.timer
systemctl --user list-timers scope-deployment-watcher.timer
systemctl --user status scope-deployment-watcher.service
```

Confirm the timer's next run, the service's last exit status, and a fresh
`SCOPE_DEPLOYMENT_WATCHER_HEARTBEAT` repository variable. Dispatch `Deployment
watcher heartbeat` once to verify the external observer. Its test suite exercises
missing/stale heartbeat alerts with mocked GitHub calls, without sending test
notifications.

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
