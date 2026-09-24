# Deployment pipeline

The September 2026 audit found delayed GitHub schedule dispatch, a long CLI
validation tail before image packaging, repeated smoke-tool compilation, and
serial provider teardown waits. The last five runs also exposed missing Git
prerequisites and differences between development smoke tests and the compiled
web runtime. The decisions and observed timings are in the
[deployment plan](https://web-production-4f13c9.up.railway.app/plans/scopevcs.com/scope-deployment-decisions-20260924).

## Validation and preparation

Planning selects components without waiting for policy and operations checks.
Those checks run as separate required jobs in both CI and Release. The required
PR check rejects either job failing, being skipped, or being cancelled.

Release validates server components separately from CLI and integration work.
Image preparation waits for backend, web, and media image validation, while CLI
and integration checks continue. Staging still waits for all selected validation,
policy, operations, production readiness, migration preflight, image preparation,
and smoke tools. A faster server build cannot bypass a failed CLI check.

Smoke tools build alongside validation. Their artifact records source revision,
Rust toolchain, and archive SHA-256. A resumed smoke run downloads and verifies
the original artifact instead of recompiling it. Staging verifies it again before
extraction. Artifacts are retained for 90 days; missing or mismatched evidence
fails explicitly.

Images package with at most two concurrent BuildKit solves on the four-vCPU
runner. Each component writes a separate manifest fragment; aggregation rejects
missing, duplicate, or inconsistent components. Registry build caches retain
stable dependency and pinned Git layers. An explicit UTC week cache input refreshes
apt dependencies, and changes to base digests, package inputs, or the Git source
invalidate the relevant layers. Source revision metadata comes after dependency
installation so each commit no longer invalidates apt work.

This increases concurrent runner and registry use. It preserves the existing
scans, immutable image checks, staging smoke, writer fences, and final observation
period. The measured savings from separate stages must not be added together;
release wall time is determined by whichever prerequisite finishes last.

## Earlier failure detection

A read-only preflight compares production receipts with live image, configuration,
and health evidence against each component's recorded deployed revision, then
audits database role invariants. It runs before staging. Proposed configuration
changes are checked during activation, not mistaken for existing production drift.
Exact role grants are compared only while the grant policy and schema match the
existing baseline; pending changes retain the role, ownership, and ledger checks.
Interrupted cutovers use their existing recovery checks because their writers are
intentionally fenced. Migration planning and the checks immediately before closing
writers remain in place.

PR integration checks now build the production web runtime and exercise it behind
an HTTPS-terminating proxy. Same-origin requests must pass the origin guard and
hostile origins must fail. A focused navigation test delays initial repository
reconciliation. Browser interception reads the compiled server-function manifest;
staging extracts it from the exact live web image rather than maintaining a
second list of function IDs.

## Activation and retries

After writer closure and schema verification, cache and media API activation can
run together, followed by the two workers. API readiness still precedes router
activation. Each component keeps separate deployment evidence. Predecessor removal
is checked at a shared bounded barrier after activation instead of blocking each
next service. The parent waits for both concurrent children before failure cleanup.
The unfenced rolling path retains sequential activation.

If the original staging job passed, requesting smoke resume does not repeat it.
Reuse still requires trusted main preparation, the validation gate, exact images,
and unchanged relevant schema, configuration, and smoke inputs. Failed smoke
continues through the explicit resume path; it cannot be treated as successful.

## Dispatch and supervision

The deployment watcher is the sole daily scheduler; GitHub cron is removed.
[The watcher operations guide](../deploy/automation/OPERATIONS.md) describes the
dispatch intent, alerting, correction chains, and the machine cutover that
keeps cron and the scheduler from running together.

Validate the next staging transition and compare its per-job timing with the audit
before claiming a production speedup. Local provider simulations cover failure and
recovery behavior, but they do not measure Railway's real activation latency.
