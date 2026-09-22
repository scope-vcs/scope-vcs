# Deployment watcher alerts

After a complete successful poll, Surface writes a UTC timestamp to the repository
Actions variable `SCOPE_DEPLOYMENT_WATCHER_HEARTBEAT` through its existing `gh`
login. Failed or incomplete polls do not renew the heartbeat.

The `Deployment watcher heartbeat` workflow runs outside Surface every fifteen
minutes. It opens one issue assigned to `adamblumoff` when the timestamp is missing,
invalid, or more than twenty minutes old. A recent timestamp closes that outage
issue. It also fails the workflow so GitHub Actions notifications can report the
failure. GitHub controls delivery according to the recipient's notification
settings. Scheduled Actions can be delayed or dropped, so this is a best-effort
external alert, not a twenty-minute delivery guarantee.

The workflow runs a Python check on a standard runner. It does not build the
application, run CI after merges, or launch another deployment agent. Its schedule
adds roughly 96 short runner jobs per day; actual billed time depends on GitHub's
runner billing minimum and repository allowance.

`heartbeat.alert(release_id, reason)` creates one assigned issue per release after
bounded recovery fails. The watcher records the returned URL only after GitHub
accepts the request and retries failed requests. Supported reason codes are
`attempts_exhausted`, `deadline_exceeded`, `agent_unavailable`, `approval_required`,
and `verification_failed`. Raw provider output is never copied into public issues.
Release alerts link to the release and direct the maintainer to Surface's watcher
state and T3 conversation for details. The optional `recoveries`, `thread_id`, and
`provider` arguments include the number of recovery attempts and the exact
conversation to inspect. They accept only bounded structured values. Closing a
release issue does not create a new alert for that same release.

Install `heartbeat.py` alongside `deployment_watcher.py` on Surface. The watcher
needs its existing repository write access to update Actions variables and create
assigned issues. The external workflow only needs repository content read and
issue write permissions. No AWS credentials or new secrets are required. The
existing AWS security email topic was considered, but the available AWS session
only permits metadata audits; adding an alarm or a publisher would require an
authenticated infrastructure administrator.

Validate locally with `python3 -m unittest discover -s deploy/automation -p 'test_heartbeat.py'`.
After deploying the watcher and pushing the workflow to main, dispatch
`deployment-watcher-heartbeat.yml` once and confirm it passes with a recent
repository heartbeat. Watcher unit tests exercise outage creation, deduplication,
recovery closure, failed publication, and provider-error redaction without sending
test alerts to the maintainer.
