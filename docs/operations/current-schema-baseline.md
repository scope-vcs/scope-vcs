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
inventory. Any other ledger — the previous chain, an older chain, an unknown
entry, a gap — is rejected by `plan`, `preflight` and `apply` with
`Scope metadata migration ledger is not a canonical prefix: expected [...],
found [...]`, without touching the ledger, schema or business rows.

Every environment adopted the baseline through a one-time bridge that shipped
from `7b5cc9f7` until its removal. A backup taken before the bridge restores
with revision `26619ef4`'s maintenance binary up to
`m0083_request_workflow_changes`, then needs a bridging revision's maintenance
binary to replace that ledger with the baseline before this binary accepts it.

## Migration definitions are immutable

`dev/checks/policy` verifies the SHA-256 digests in
`crates/scope-postgres/src/migrations/sources.lock.json`. The baseline SQL and
the numbered migration sources must retain their bytes. Pull
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
requires an empty schema. For a ledger holding only the baseline, it compares the actual schema with the baseline instantiated in a
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
schema checks, ledger rejection, schema drift, rollback, repeated application, writer-fence enforcement and run-state parity:

```sh
SCOPE_TEST_DATABASE_URL=postgres://scope:scope@127.0.0.1:5432/scope_test \
  CARGO_BUILD_JOBS=2 cargo test -p scope-postgres migration
```
