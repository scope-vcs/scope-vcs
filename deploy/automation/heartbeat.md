# Deployment supervision alerts

The external GitHub observer runs Python every fifteen minutes. It checks main's
dated daily Release independently and alerts after five minutes without a timely
start. It also checks bounded-session progress, deadlines, retained repair
ownership, and active or failed release attempts not admitted by supervision.
An idle daytime is healthy even when its last progress is hours old. There is
no all-day heartbeat requirement or recurring model poll.

A completed session cycle publishes bounded JSON to
`SCOPE_DEPLOYMENT_SESSION_STATUS`: lifecycle status, deadline, progress time,
active releases and repair owners, activation date, installation boundary, daily audit boundary, and
recent observed run attempts. Failed cycles save the failing operation locally
and do not renew successful progress. A crash restart retains the deadline.
A running session more than twenty minutes without completed progress, a deadline
exceeded by more than five minutes, or an unconfirmed owner after escalation is
unhealthy. The observer creates one assigned outage issue and closes it after a
healthy check. Dated start issues remain open for investigation; a late run does
not erase the failure. A main Release without supervision after twenty minutes
also alerts, including a missed event for a completed failed run or new attempt.
Recent forwarder run titles provide durable GitHub event receipts for completed
retries whose original creation date lies outside the recent Release scan.
The observer validates the forwarder identity and independently reads the Release.
Completed attempts that predate the watcher's preserved installation boundary
are excluded. Active attempts remain eligible regardless of age, and reruns
started after installation must be observed. Confirmed session cleanup publishes
its stopped state; failed cleanup publishes the failure and retained ownership.

During installation, the existing minute timer keeps publishing
`SCOPE_DEPLOYMENT_WATCHER_HEARTBEAT`. Until the first bounded session publishes its
status, the external workflow checks that operating owner's timestamp. Rollback
removes the session-status variable before restoring the timer. This transition
keeps external observation available while tunnel and signing setup are pending.

The workflow runs no model, build, or post-merge CI. GitHub may delay scheduled
jobs and controls notifications through the recipient's settings; alerts are
best effort. Its schedule is roughly 96 short runner jobs per day, with billing
subject to GitHub's runner minimum and repository allowance.

Release escalation uses `heartbeat.alert` and one marker per release, even if
its earlier issue was closed. Reasons include exhausted recovery, deadline,
agent availability, approval, and failed verification. Context is restricted to
recovery count, provider, numeric run ID, and T3 UUID. Raw provider output,
credentials, and webhook signing values never appear in public issues.

The host uses its existing GitHub login for dispatch, Actions-variable updates,
and assigned issues. The observer requires Actions read, content read, and issue write.
The separate event forwarder requires Actions read and a shared HMAC secret.
T3 Connect authenticates that signature before dispatching a task prompt; see
[OPERATIONS.md](OPERATIONS.md) for secret setup and safe owner cutover.

After installation, manually run the external heartbeat workflow and verify its
actual GitHub result. Local tests cover idle and active health, missed starts,
event gaps, deadlines, issue deduplication, recovery closure, and redaction with
mocked GitHub calls, without sending test alerts.
