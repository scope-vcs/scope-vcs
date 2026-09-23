# PostgreSQL service roles

`runtime-roles.mjs` renders the reviewed role policy as transactional psql input. It never connects to a database. Use a dedicated Scope database with application objects in `public`. Unknown tables or additional application schemas fail the transaction. A missing expected table fails its grant.

| Service | Database login | Access |
| --- | --- | --- |
| API | `scope_api` | Repository, collaboration, identity, runs and media admission data. No cache object tables. |
| Run worker | `scope_run_worker` | Run, outbox, compaction, dependency and cleanup writes; repository facts, membership, history and request/revision/user reads. No CLI sessions, identity credentials, cache objects or media tables. |
| Cache | `scope_cache` | Cache tables; reads runs, jobs and attempts to revalidate signed grants. |
| Media API | `scope_media_api` | Upload parts and abandoned objects, attachment row locks, media manifests and authorization reads. |
| Media worker | `scope_media_worker` | Media processing/cleanup tables; reads repositories and requests. |
| Private maintenance | `scope_migrator` | Database/schema/object owner. Can migrate and restore; inherits `pg_signal_backend` to drain runtime writer sessions. |

Every runtime login can read the migration ledger and use `public`. None can create permanent or temporary tables, change schema, create roles, bypass row security, become the migration owner or inherit another role. Only API/run worker can advance the run sequence. New tables and functions receive no runtime access until this explicit policy is updated. The policy removes stale table, column, sequence and routine grants, including grants through `PUBLIC`. Extension-owned routines retain their extension privileges and ownership; Scope application routines receive no runtime EXECUTE grant.

## Initial cutover

1. Prepare and test a restored copy first. Record a tested recovery point for metadata, objects and encryption keys. Record the current service settings without printing secrets in CI logs.
2. Review the table grants against the release's database callers. The mapping is intentionally explicit. The cache service's `authorize_cache_grant` reads `scope_runs`, `scope_run_jobs` and `scope_run_attempts`. Media authorization reads repository membership and request invitees; media mutations use `scope_metadata_locks`. Worker responsibilities include more than run dispatch.
3. Pause writers. Render `node deploy/postgres/runtime-roles.mjs` to an access-restricted temporary SQL file. Apply it using `psql -X -v ON_ERROR_STOP=1` through private maintenance as the database administrator. This bootstrap requires role administration and object ownership transfer authority. No public connection string belongs in CI.
4. New logins have no password. Set separate generated passwords through the private administrator connection using psql's `\password role_name`, then configure each service's private `DATABASE_URL` with its own login. Put `scope_migrator` credentials only in private maintenance. Preserve existing encryption keys. Require SCRAM authentication for database logins; do not use trust authentication in deployed environments.
5. Restart the paused services with their new credentials. Check readiness, browser and CLI sign-in, Git clone/push, request mutations, media upload/read/processing/deletion, cache restore/upload/GC, run launch/logging/cancel/retry, compaction and content cleanup. Complete a release and staging baseline restore using the migration login. These application canaries are required before declaring live cutover complete; the isolated ACL test does not cover every application transaction.
6. Verify the old runtime credential is absent from services, replicas and scheduled jobs, then retire it. Do not put the administrator credential back into service settings to bypass a failed grant. Correct the reviewed grant and repeat the failed canary while writers remain paused.

The migration role can signal other non-superuser sessions through `pg_signal_backend`. Keep it private and use the existing writer-drain selection logic. Runtime roles receive no signal capability. Database-owner maintenance is intentionally privileged; it must never run untrusted job instructions.

## Every migration and restore

Run migrations as `scope_migrator`. After applying migrations or restoring the public schema, render and apply `node deploy/postgres/runtime-roles.mjs --grants-only` using that same private login before restarting writers. This mode neither creates roles nor changes role attributes/memberships. It reapplies ownership, explicit ACLs and default-deny policy. A failed grant refresh fails maintenance and must keep writers stopped.

Schema additions must update the inventory and service grants in the same change.
A grants-only policy change also needs a migration to enter the writer cutover,
or an explicit grant refresh with writers paused. Exact-ledger releases leave
read-only preflight unchanged. Never replace this with default grants on all future tables. Restore without source ownership/ACLs, restore into the migration owner's schema, and reapply this policy. After a restore, repeat readiness and permission-denial checks before resuming writers.

`m0059_worker_history_permissions` is a ledger marker for the worker history
grants. It changes no schema. Its pending state sends the release through the
paused maintenance cutover, whose apply wrapper refreshes this role policy
before writers reopen.

Role names are cluster-wide. Use separate PostgreSQL instances for separate environments, and do not reuse these logins for unrelated databases. Bootstrap rejects role memberships except the migration role's `pg_signal_backend` membership. It does not inventory permissions in other databases; check and remove such access before cutover. Preserve a separately controlled administrator recovery path.

## Local verification

Run `node --test deploy/postgres/runtime-roles.test.mjs`. The test creates and removes its own local cluster, loads the real baseline and table-adding migration SQL, and connects as each service login. It verifies every effective table grant, runs the Rust outbox history rebuild as the worker login, denies direct history-entry deletion and unrelated mutations or reads, checks schema restore and grant refresh, checks future objects default to denied, removes a stale column grant, and confirms the migration login can terminate a runtime session. It also rejects unexpected tables and inherited role access. Cargo is required for the worker rebuild check.

PostgreSQL 18 server tools are required at `/usr/lib/postgresql/18/bin`; set `SCOPE_TEST_POSTGRES_BIN` for another installation. The test never reads `DATABASE_URL` and never uses the machine's running PostgreSQL instance.
