# Current schema baseline

The migration inventory starts at `m0084_current_schema_baseline` and continues
with the numbered migrations after it. `m0084` installs
`crates/scope-postgres/src/migrations/m0084_current_schema_baseline.sql` into an
empty schema; every later change is a numbered migration.

The baseline is the schema produced by the previous inventory,
`m0042_current_schema_baseline` through `m0083_request_workflow_changes`, at
revision `26619ef4bb5ca7fe8bb938ac3f5ca0db73c41282`. It was captured with
PostgreSQL 18.6 `pg_dump --schema-only` from a fresh database migrated by that
revision. None of those migrations seeded rows into a fresh database; their data
statements only rewrote existing rows, so the baseline carries no data.

A database is recognized only when its applied ledger is a prefix of that
inventory, or exactly the previous inventory awaiting the bridge below. Any
other ledger — an older chain, a partial previous chain, an unknown entry, a gap
— is rejected by `plan`, `preflight` and `apply` with
`Scope metadata migration ledger is not a canonical prefix: expected [...],
found [...]`, without touching the ledger, schema or business rows. The
diagnostic names the pinned revision that can advance a partial previous chain.

## Baseline bridge

A database whose ledger is exactly the 42 names in
`crates/scope-postgres/src/migrations/m0084_bridged_ledger.txt` adopts the new
baseline once, through the normal maintenance cutover:

- `plan` reports that ledger as `applied` with `m0084_current_schema_baseline`
  pending, `exact: false` and `metadataRestoreSafe: true`. Ordinary startup
  refuses it like any other pending migration.
- `preflight` compares the live schema with the new baseline while writers stay
  online, inside a transaction that always rolls back.
- `apply` takes the writer fence and migration lock, locks the ledger table,
  rereads the exact bridged ledger, compares the schema again, then replaces the
  42 ledger rows with `m0084_current_schema_baseline` in the same transaction.
  Business rows, sequence counters, ownership and grants are untouched.

Schema drift fails both commands with `schema drift` and leaves the old ledger
in place. Production must be on the bridged ledger when the bridging release
runs; a database behind it first advances with the pinned revision's
maintenance binary. Keep that binary available for as long as backups taken
before the bridge must remain restorable. A restored pre-bridge backup at the
exact bridged ledger can be bridged by the current binary; a backup behind it
must first be advanced by the pinned binary.

The bridge counts as a pending migration, so the release enters the paused
maintenance cutover and refreshes runtime grants before writers reopen. The
staging baseline snapshot stays keyed to the production ledger that existed
before the bridge.

## Migration definitions are immutable

`dev/checks/policy` verifies the SHA-256 digests in
`crates/scope-postgres/src/migrations/sources.lock.json`. The baseline SQL, the
bridged ledger and the numbered migration sources must retain their bytes. Pull
request checks also compare existing lock entries with the target branch, so
updating a checksum alongside an old migration is rejected. Definitions retired
by a squash are listed in `.github/scripts/check-migration-immutability.mjs`;
a retired name can never be locked again.

Change an existing database with a new numbered migration and add its digest
to the lock. Keep migration SQL self-contained; calling mutable application
helpers can change historical migration behavior without changing its source.
Do not edit existing migrations or regenerate their lock entries to make a
schema check pass.

## Maintenance behavior

`scope-maintenance plan` reports the ledger and the pending migrations. Ordinary
startup refuses anything short of the exact expected inventory. The current
binary never fills gaps or resets a database to make its ledger fit.

`scope-maintenance preflight` runs the same planning while writers remain
online, inside a transaction that always rolls back. For an empty ledger it
requires an empty schema. For a ledger holding only the baseline, or the bridged
ledger, it compares the actual schema with the baseline instantiated in a
separate schema on the same PostgreSQL server. The comparison covers logical
column order and types, defaults, constraints, indexes, sequence definitions and
ownership links, functions, triggers, views, rules, policies and schema-local
types. PostgreSQL parses CHECK expressions and partial-index predicates through
temporary views so equivalent casts produced by a dump/restore do not look like
schema drift. For PostgreSQL 18 NOT NULL constraints, it compares the column
definition and flags while retaining names inherited from historical column and
table renames. The views read no business rows, and the temporary comparison
schema is dropped before preflight returns. The comparison excludes
environment-specific role ownership and grants, and allows the baseline's
`public.pg_trgm` extension objects.

Preflight does not replay data migrations or claim to validate every schema
object once later migrations have applied; `plan` remains a ledger-only
operation for recovery after a migration has committed.

`scope-maintenance apply` acquires the exclusive metadata writer fence and the
migration lock, then applies every pending migration in one transaction. A
failed transaction restores the ledger, schema and business rows. If the commit
response is lost, read the ledger again: the previous inventory can retry and
the new inventory can continue forward activation. Unknown states leave writers
closed — follow the deployment recovery workflow in
[maintenance migration recovery](../maintenance-cutovers.md) instead of manually
editing ledger rows.

## Run states are owned by the domain

`crates/scope-domain/src/runs/` defines the run, job, attempt and step states.
`as_str` is their persisted spelling: `crates/scope-postgres/src/db/run_state_sql.rs`
builds every SQL state set from it, and a migration test compares the state
strings allowed by `scope_runs_values`, `scope_run_jobs_values`,
`scope_run_attempts_values` and `scope_run_attempt_steps_values` with the enum
variants. A new state therefore needs both a variant and a migration.

## Local verification

The PostgreSQL tests cover fresh initialization, empty-schema and baseline
schema checks, ledger rejection, the bridge and its drift refusal, schema drift,
rollback, repeated application, writer-fence enforcement and run-state parity:

```sh
SCOPE_TEST_DATABASE_URL=postgres://scope:scope@127.0.0.1:5432/scope_test \
  CARGO_BUILD_JOBS=2 cargo test -p scope-postgres migration
```
