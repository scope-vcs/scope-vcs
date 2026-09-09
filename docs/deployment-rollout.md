# Deployment rollout record

The Railway environment and naming transition completed on September 9, 2026, after [PR #302](https://github.com/scope-vcs/scope-vcs/pull/302) merged. [Maintenance cutovers](maintenance-cutovers.md) describes recurring releases. The [deployment manifest](../.github/deployment-services.json) owns the live target IDs and domains.

## Retained infrastructure

The [scope-vcs Railway project](https://railway.com/project/45dd67fa-6d69-48ad-9680-1313d41b4490) has two persistent environments:

| Environment | Exact environment ID | Database volume instance | Volume label |
| --- | --- | --- | --- |
| production | `2f21f6b6-6817-4338-9e08-cf18e78b8f46` | `d849de88-290f-4f68-a6bb-33a8bac7c0fd` | `scope-postgres-volume` |
| staging | `f6cdb01a-7a42-40ff-b799-bffe254191b2` | `5bbc3c7c-a680-49fd-ac95-1f51bca02364` | `scope-postgres-staging-volume` |

Staging is the former release-proof environment. Railway requires unique volume labels within a project, so its volume uses an explicit staging label. Retained database mounts, deployment IDs, three logical buckets and six physical bucket mappings were unchanged by the transition. Both databases retained the same 42 applied migrations through `m0042_request_media`; staging retained `dev/public-demo` and `dev/update-demo`.

Service names are `scope-api`, `scope-run-worker`, `scope-git-router`, `scope-media-api`, `scope-media-worker`, `scope-cache`, `scope-web`, `scope-cli-downloads` and `scope-postgres`. Staging domains use `scope-<role>-staging.up.railway.app` for API, web, cache, Git router and media API. Dependent variables were updated and read back without restarting production. Stable private DNS names were preserved where Railway retained them.

Production and staging keep their existing GitHub credentials and `main` branch policies. The raw pending Railway configuration patches were empty after the transition. The earlier MCP report of 113 production changes was a sparse-patch display error, not evidence of deleted credentials. No production configuration was rebuilt to clear that display.

## Retired resources and recovery evidence

The following exact environments were deleted after database restores, object checksums, current fixture identities and credential consumers were verified:

| Retired environment | Exact environment ID | Preserved data |
| --- | --- | --- |
| old staging | `8743c2e9-5d9b-4161-a047-9617e6d199b9` | Two demo repositories, object buckets, attached database and detached PostgreSQL cluster |
| media-proof | `93ae668a-a0e3-4c57-b6c0-4ad0592ab68c` | Two demo repositories, object buckets, attached database and empty detached volume |
| loadtest-push-persistence | `144cb440-a4f4-40c3-b079-00351895ca11` | Seventeen generated loadtest repositories, object bucket, attached database and empty detached volume |

Railway retained the deleted old staging name on its soft-deleted record. Renaming that record to `retired-staging-20260909` released the name; its deletion state remained intact. No environment was restored to perform the rename.

Recovery evidence is in the restricted local archive `~/.local/state/scope-vcs/deployment-archive-20260909`. It includes verified database dumps, all 585 retired bucket objects, detached-volume exports and readable configuration. The formerly unclassified 876 MB detached volume contained a PostgreSQL 18 cluster; its copied cluster started locally and contained no repositories. Temporary archival services, access keys and mounts were removed afterward.

A separate production logical backup under `retained/production-recovery-20260909` restored successfully to an isolated PostgreSQL 18 instance. All 68 table counts, repository count and 42 migration entries matched production. Three sealed media settings remain provider-only and must be preserved or recovered from their original secure source; they are not present as plaintext in the archive. Railway also registered snapshot `246a8563-09fd-4148-9eee-64b6decc97f7`. Its restore was not tested; the verified recovery artifact is the logical backup. No recurring backup policy was changed.

The shared Northflank runner job and the separate `scope-git-cloud-lab` project were preserved. No logical project bucket or retained database volume was deleted.

## Deployment records and release verification

Six successful component receipts were rekeyed to `run-worker`, `git-router`, `media-api`, `media-worker`, `cli-downloads` and `checks-image`, preserving their source SHA, provider deployment ID and artifact evidence. GitHub cleanup retired 44 obsolete environment metadata records. All 813 previously archived deployment records remained available afterward, alongside the six canonical copies. Historical `production/cutover` and the retained provider alias `scope-vcs / production` remain for audit purposes; new maintenance operations use `production/maintenance`.

Six obsolete workflow definitions were disabled after their files were removed from `main`: runner performance diagnostic, runner performance experiment, private release image diagnostic, staging cache database experiment, public release image retirement, and production readiness bootstrap.

The superseded legacy release was cancelled before production activation after the new orchestration merged. Its staging cleanup successfully stopped writers and revoked the temporary token. Its observed elapsed times were 53m29s for the parent run and 49m33s for the staging run; these cancelled runs do not establish a successful-release duration or billed runner cost.

The first new [Release attempt](https://github.com/scope-vcs/scope-vcs/actions/runs/34383599041) stopped before staging deployment because the baseline script required a private GitHub repository. Scope is public. No staging restore, migration, or production activation ran; cleanup fenced staging writers and revoked the temporary token. [PR #304](https://github.com/scope-vcs/scope-vcs/pull/304) replaced that assumption with authenticated encrypted snapshots and verified the no-migration path without archive access. The staging GitHub environment now holds the snapshot encryption key.

That failed attempt took 19m54s wall time and 48m26s summed observed job time. Its staging phase took 6m38s, including 5m05s building smoke tools. It produced seven pinned application image digests and built the checks image. These are observed execution measurements, not billed runner costs or a successful-release benchmark.

The controlled follow-up's production preflight found 42 applied migrations, zero pending migrations, and an exact ledger. This release exercises the ordinary deployment path. Focused HTTP, interruption, provider, and PostgreSQL tests cover maintenance and encrypted baseline restoration.

The [next attempt](https://github.com/scope-vcs/scope-vcs/actions/runs/34388134239) passed baseline reconciliation, then failed because staging started the Git router before restarting API replicas. Router readiness could not resolve the stopped API's private address. Production was skipped, and cleanup fencing and token revocation passed. [PR #305](https://github.com/scope-vcs/scope-vcs/pull/305) corrects staging activation order and the same dependency in fresh production bootstrap. New provider tests reproduce the failure against the old order.

This attempt took 19m43s wall time and 45m17s summed observed job time. Staging took 5m24s, including 2m13s building smoke tools. It again prepared seven pinned application image digests and the checks image. These failed-run measurements do not establish the final release duration.

The [controlled release](https://github.com/scope-vcs/scope-vcs/actions/runs/34391919309), pinned to `a2d2119d6f06822f3f369a0223da5bf007272d09`, passed baseline reconciliation, deployed all seven staging participants once, and passed browser and Git smoke tests. Staging cleanup fenced writers and revoked the temporary token. The staging job took 9m44s, including 2m07s building tools and 6m58s deploying and running smoke tests. Production preflight again found 42 applied migrations and zero pending migrations.

Its first production attempt activated the Git router successfully, then a Railway API read timed out before the remaining backend components activated. All nine production services still reported healthy replicas, and six public probes returned HTTP 200 with the expected response bodies. GitHub accepted a failed-job retry but did not admit it or create new jobs; its cancellation API rejected cancellation because the retry had not queued.

The [prepared-release replay](https://github.com/scope-vcs/scope-vcs/actions/runs/34395332911) verified the original validation and staging evidence and reused the same artifacts. Its production backend and web deployments passed, followed by the aggregate production health check. The scheduler was re-enabled, and the daily gate returned `due=false` against the successful release on September 9 at 19:49 UTC.

The normal [CLI follow-up](https://github.com/scope-vcs/scope-vcs/actions/runs/34397367793) completed CLI publishing, which the failed backend attempt had skipped and application replay does not include. All 15 selected jobs passed, including platform builds, native checks, seeded integration, CLI deployment, and the final release verification. It ran no staging, backend, or web deployment. The exact CLI deployment `cc728f01-73b3-488f-8729-6985802ec5ad` reported `SUCCESS` with one running replica. The follow-up took 11m50s wall time and 22m04s summed observed job time, including 1m11s deploying CLI downloads. The daily gate again returned `due=false` at 20:01 UTC, and Release remained active.

The successful application replay took 19m52s wall time and 17m50s summed observed job time. Backend deployment took 10m47s, web deployment 2m27s, and the final health job 13s. It reused seven pinned image digests and ran no new image builds or staging deployment. All nine production services reported successful deployments with one running replica and no crashed replicas; all six public response-body probes passed after the aggregate result. These timings describe recovery, not a clean full-release benchmark.

- [x] Merge orchestration changes and verify CI and review fixes.
- [x] Archive retired data and verify restoration and ownership.
- [x] Retire three exact environments and verify retained storage.
- [x] Normalize service, environment, domain and receipt names.
- [x] Retire obsolete GitHub environment metadata while preserving deployment history.
- [x] Verify the controlled release, pinned to `a2d2119d6f06822f3f369a0223da5bf007272d09`, through staging smoke and the prepared replay's production aggregate health result.
- [x] Complete the CLI publishing follow-up.
- [x] Restore scheduled releases and verify the daily gate skips after today's success.

GitHub retained two unadmitted queue entries at the final check: run `34387523541` and the retry of `34391919309`. Neither had new jobs. Cancellation returned HTTP 409 because GitHub had not queued them internally. Their state is archived; cancel these obsolete runs if GitHub later admits them.

Future experiments follow the [owner and expiry policy](railway-experiments.md). An independent hourly audit checks the complete provider inventory against the registry. Expired or unmanaged experiments fail the audit; cleanup still requires data review and archival.

The first [GitHub Actions audit](https://github.com/scope-vcs/scope-vcs/actions/runs/34386751194) passed against the live provider on September 9 at 18:04 UTC. It found two protected environments, zero experiments, and no issues. The hourly audit is active.

The previous billing period, August 9 through September 9, recorded $14.4886 of Railway project resource usage. The new period had recorded $0.0700 at the audit snapshot. Raw environment measurements and service cost breakdowns are archived under `provider-transition`. Compare a full subsequent billing interval before claiming savings; neither partial-period usage nor cancelled workflow elapsed time is a savings estimate.
