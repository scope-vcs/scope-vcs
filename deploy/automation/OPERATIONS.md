# Release supervision

Surface runs `scope-deployment-watcher.timer` every minute, all day. The watcher
starts a T3 agent as soon as a main Release workflow appears, including queued
validation and failures before staging. PR checks remain the only merge-time CI.
Release retains its existing daily schedule and manual dispatch.
The Scope mirror also runs checks on requests and manual starts, not pushes to main.

The watcher tracks releases in
`~/.local/state/scope-deployment-watcher/supervision.json`. Successful dispatch
means monitoring, not completion. GitHub's successful `Verify and record release`
job supplies the independent production evidence. A successful no-change run must
explicitly skip all deployment jobs. Corrective runs require an agent-written
mapping in the incident's `receipts` file and their own successful production
verification. Unrelated successful runs do not silently resolve older failures.

One conversation owns an open investigation. Newly discovered releases appear in
its `inboxes` file, which the prompt requires the agent to read each monitoring
cycle. Repairs use PRs with squash auto-merge after `Required PR checks` passes.
They never bypass protection or push directly to main. An interrupted worktree is
preserved for the replacement provider.

The supervisor allows three agent recoveries after the initial dispatch. It
resumes a prematurely finished agent after two minutes, falls back from Codex to
Claude on an error or repeated early exit, and interrupts an agent with no
activity for twenty minutes. It waits for the old agent to stop before resuming.
Unanswered approval/input requests escalate after ten minutes. An investigation
has a four-hour recovery limit. The agent is instructed to cap corrective
deployments at three. An assigned GitHub issue records exhausted recovery or a
reported blocker. Escalation stops automatic repair; an agent that cannot be
stopped retains ownership, preventing overlapping repairs.

See [heartbeat.md](heartbeat.md) for the external missing-heartbeat alert. It uses
the existing GitHub login and sends no build jobs. GitHub scheduling and issue
notification preferences affect alert delivery; this is not a paging SLA.

## Install on Surface

Verify hostname `adam-blumoff-surface-book-2` and fleet identity `surface` before
changing files. Run the tests from the repository root:

```sh
python3 -B -m unittest discover -s deploy/automation -p 'test_*.py'
```

Stop the watcher timer and wait for its current service invocation to finish.
Inspect any existing T3 deployment monitor before initializing new supervision
state. Do not start a second agent while an old one is repairing a release.
Install `deployment_watcher.py`, `deployment_policy.py`, `deployment_runtime.py`,
and `heartbeat.py` in `~/.local/share/scope-automation/release-supervisor/`.
Install the `.service` and `.timer` files in `~/.config/systemd/user/`.

```sh
systemctl --user daemon-reload
python3 -B ~/.local/share/scope-automation/release-supervisor/deployment_watcher.py --initialize --dry-run
python3 -B ~/.local/share/scope-automation/release-supervisor/deployment_watcher.py --initialize
systemctl --user enable --now scope-deployment-watcher.timer
```

Initialization establishes a boundary for completed historical runs, so deployment
failures already repaired before installation are not reactivated. It still adopts
unfinished releases. Subsequent invocations never initialize state implicitly.
The previous watcher's `state.json` is historical operational evidence and is not
read by this supervisor. A missing/corrupt new state fails the poll and eventually
raises the external heartbeat alert.

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
