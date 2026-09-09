# Deployment rollout checklist

Provider rollout is pending the release-orchestration changes reaching `main`. Keep the existing Railway names while legacy workflows can still run. This checklist covers the one-time transition; [maintenance cutovers](maintenance-cutovers.md) describes recurring releases.

The target is the Railway [scope-vcs project](https://railway.com/project/45dd67fa-6d69-48ad-9680-1313d41b4490). Use exact IDs from [the deployment manifest](../.github/deployment-services.json), not similarly named resources in other projects.

## Preserve data and resolve pending changes

- [ ] Recheck active release, proof and experiment runs. Let them finish or recover before changing their targets. Check the durable production cutover record is terminal.
- [ ] Export a sanitized resource inventory and current deployment IDs. Preserve database dumps, nonempty object buckets, and the credentials and encryption configuration needed to restore them in restricted archival storage. Verify checksums and restore reads before deleting their source resources. Keep secrets out of tickets, workflow logs and this repository. Preserve sealed configuration in place; names-only exports cannot recreate sealed values.
- [ ] Inspect production patch `2b89ea3e-a58d-4e0e-b127-b300e8fe2bcf` through the raw GraphQL patch before changing it. The September 9 MCP display reported 113 changes, but the raw patch was empty `{}`. The display compared a sparse patch as a complete configuration and falsely reported variable removals. Archive the raw patch and preserve live and sealed configuration. Do not accept or rebuild production configuration to resolve a display artifact.
- [ ] Record the available recovery point before the first maintenance release. The audit found no scheduled volume backups; production's latest snapshot was August 25. A backup's existence alone does not establish restoration works. This rollout does not introduce a new recurring backup policy.

## Retire the old experiments

The September 9 read-only database audit found only `dev/public-demo` and `dev/update-demo` in old staging and media-proof, matching [seed fixtures](../api/src/dev/seed.rs). Each had three users, six requests and no run jobs. Loadtest held 17 generated `loadtest-mixed-*` repositories owned by `loadtest`, one user and no requests or run jobs. Recheck those findings before deletion.

| Retire environment | Exact environment ID | Attached database volume instance | Other volume instance |
| --- | --- | --- | --- |
| `staging` | `8743c2e9-5d9b-4161-a047-9617e6d199b9` | `cb8f1e44-21cf-4278-a773-b3d06b7a37ad` | `ac1e4646-9c1d-4cdd-a632-bb597765c1e8`, detached, about 876 MB |
| `media-proof` | `93ae668a-a0e3-4c57-b6c0-4ad0592ab68c` | `f0b4097c-c78f-4276-ba33-9e35e7ed0543` | `aa3f4d94-ce4f-47f2-af25-5f48a3041f3e`, detached, about 7.8 MB |
| `loadtest-push-persistence` | `144cb440-a4f4-40c3-b079-00351895ca11` | `cd4a7cc7-ac75-468d-ac55-7d987ca50aa3` | `c75e8d10-dfd3-44ac-802e-05e5eaf59258`, detached, about 7.8 MB |

- [ ] Classify and preserve detached volumes. The 876 MB old staging volume's contents are unknown. Do not delete that volume or its containing environment while it remains unclassified. If it must survive retirement, move or archive it and verify restoration first.
- [ ] Archive nonempty buckets using their physical names. Old staging had 56 blob objects and five cache objects totaling about 1.53 GB; media-proof had 44 blob objects; loadtest had 480 blob objects. Audit actual application bucket references as well as provider resource ownership. Logical bucket and volume IDs repeat across environments; delete only environment instances, never shared project resources.
- [ ] Preserve the Northflank `scope-runners` / `scope-cloud-runs` job. Media-proof and loadtest reference it alongside retained release-proof. Their cloud runs were disabled at audit time. Remove only credentials and jobs proven exclusive to retired targets.

## Switch names after merge

1. Merge the orchestration changes, then hold scheduled/manual releases until this provider transition finishes. Confirm no legacy workflow run remains active. Do not rename services before this boundary.
2. Complete the archives and retirement checks above, then delete the three exact retired environment IDs. If old staging cannot yet be deleted because of its detached data, resolve that preservation task before reclaiming its name.
3. Keep production `2f21f6b6-6817-4338-9e08-cf18e78b8f46`. Rename retained release-proof `f6cdb01a-7a42-40ff-b799-bffe254191b2` to `staging`. Preserve its database volume instance `5bbc3c7c-a680-49fd-ac95-1f51bca02364`; preserve production instance `d849de88-290f-4f68-a6bb-33a8bac7c0fd`.
4. Rename the five project services below. Resolve variable references, private hostnames and generated `RAILWAY_SERVICE_*` dependencies before restarting consumers. Service names apply across environments. Verify external domains separately; a display-name change is not evidence that DNS changed.
5. Verify GitHub `staging` keeps its nonproduction credentials and `main` branch policy. The paginated inventory confirms it already exists; no replacement credential environment is needed. Keep production's existing `main` policy and verify Railway token scope against the retained environment ID.
6. Run `node deploy/railway/rekey-deployment-receipts.mjs` from the merged revision, inspect its dry-run plan, then repeat with `--apply`. Copy the latest successful component revisions into canonical receipt keys and verify SHA/status preservation. The new orchestration writes `production/maintenance`; the helper requires both it and historical `production/cutover` journals to be terminal before copying receipts. Keep the aggregate release and durable cutover history. Do not invent successful deployments to fill missing entries.
7. Inspect deployment planning with the rekeyed receipts, deploy once to staging and run its smoke checks. Then run the first production release and verify its aggregate result before restoring the schedule.

| Existing service | Canonical service | Exact service ID |
| --- | --- | --- |
| `scope-worker` | `scope-run-worker` | `a474a0d1-3000-4a95-9ca3-cd3a7f3ef669` |
| `scope-repo-router` | `scope-git-router` | `30f8e9d2-6d66-4f88-ad96-a65e1bba503c` |
| `scope-media` | `scope-media-api` | `6d7b48da-e52e-47bb-b4af-d6fc2d03aec3` |
| `scope-cache-service` | `scope-cache` | `668d47df-8f68-4f96-b52f-7c02324829e3` |
| `scope-cli` | `scope-cli-downloads` | `a212bccb-ed1a-4574-a031-2b0183e08a2d` |

Receipt keys change from `worker`, `router`, `media`, `mediaWorker`, and `cli` to `run-worker`, `git-router`, `media-api`, `media-worker`, and `cli-downloads`. Keep `api`, `web`, and `cache`. Confirm the helper's mapping for independently published images against the merged manifest.

## Completion evidence

- [ ] Railway has exactly `production` and `staging`, with the retained IDs above and canonical service names.
- [ ] Retired data has a verified disposition; no unclassified volume or shared external job was deleted.
- [ ] Production raw pending patch has no unintended changes; required live and sealed configuration survives.
- [ ] GitHub receipt planning uses the real latest successful revisions under canonical keys.
- [ ] Staging smoke and the first production aggregate health result pass. Scheduled releases are restored, and runtime endpoints resolve through the intended environment.

Record completion dates, archive locations and verification results in the rollout issue. Leave unchecked work explicitly pending; merging code alone does not complete this infrastructure transition.
