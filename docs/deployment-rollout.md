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

The superseded legacy release was cancelled before production activation after the new orchestration merged. Its staging cleanup successfully stopped writers and revoked the temporary token. Its observed elapsed times were 53m29s for the parent run and 49m33s for the staging run; these cancelled runs do not establish a successful-release duration or billed runner cost.

- [x] Merge orchestration changes and verify CI and review fixes.
- [x] Archive retired data and verify restoration and ownership.
- [x] Retire three exact environments and verify retained storage.
- [x] Normalize service, environment, domain and receipt names.
- [x] Retire obsolete GitHub environment metadata while preserving deployment history.
- [ ] Verify [the first Release run](https://github.com/scope-vcs/scope-vcs/actions/runs/34383599041), pinned to `f0492a4c725e929f77fde611ec5ac7d465c9add8`, through staging smoke and the production aggregate health result.
- [ ] Restore scheduled releases after that result succeeds.

Future experiments follow the [owner and expiry policy](railway-experiments.md). An independent hourly audit checks the complete provider inventory against the registry. Expired or unmanaged experiments fail the audit; cleanup still requires data review and archival.

The previous billing period, August 9 through September 9, recorded $14.4886 of Railway project resource usage. The new period had recorded $0.0700 at the audit snapshot. Raw environment measurements and service cost breakdowns are archived under `provider-transition`. Compare a full subsequent billing interval before claiming savings; neither partial-period usage nor cancelled workflow elapsed time is a savings estimate.
