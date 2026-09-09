# Current schema baseline and retained databases

The active inventory starts at `m0042_current_schema_baseline`, followed by
`m0043_retire_git_manifests`. A fresh database creates the current schema
directly. A retained database uses a one-time maintenance bridge after reaching
the exact original inventory through `m0042_request_media`.

The original-chain source is pinned at
`578bec00da088598919082b35a7153f62bf0b860`. Keep its maintenance binary, source
revision and required storage configuration with every backup that predates the
baseline. The old chain is not compiled into the current application.

## Observed retained states

Read-only inspection on September 8, 2026 found these public-schema ledgers.
All five environments reported PostgreSQL 18.6.

| Environment | Last original migration | Required preparation |
| --- | --- | --- |
| production | 42 | Verify schema and restoration proof, then bridge |
| release-proof | 42 | Verify schema and restoration proof, then bridge |
| media-proof | 42 | Verify schema and restoration proof, then bridge |
| staging | 41 | Advance to 42 with the pinned original runner |
| loadtest-push-persistence | 33 | Advance to 42 with the pinned original runner |

These observations authorize no environment writes. Before deployment, settle
which databases and backups must be retained, preserve their restore artifacts,
and rehearse their transitions. Advancing the older environments and proving
backup restoration remain deployment prerequisites. Preserve immutable images
referenced by unresolved cutovers as described in
[maintenance migration recovery](../maintenance-cutovers.md).

## Maintenance behavior

`scope-maintenance plan` reports the baseline as maintenance required when the
ledger contains exactly the original 42 migrations. Ordinary startup refuses
that plan. Older, incomplete, unknown and newer original histories are rejected
with the pinned source revision in the diagnostic. The current binary never
fills gaps or resets a database to make its ledger fit.

`scope-maintenance apply` acquires the existing exclusive metadata writer fence
and migration lock. For the exact original ledger, it locks the ledger table and
compares the schema with the baseline instantiated in a separate schema on the
same PostgreSQL server. The comparison covers logical column order and types,
defaults, constraints, indexes, sequence definitions and ownership links,
functions, triggers, views, rules, policies and schema-local types. PostgreSQL
parses CHECK expressions and partial-index predicates through temporary views
so equivalent casts produced by a dump/restore do not look like schema drift.
For PostgreSQL 18 NOT NULL constraints, it compares the column definition and
flags while retaining names inherited from historical column and table renames.
The views read no business rows. The temporary comparison schema is dropped
before the ledger changes.

The comparison excludes environment-specific role ownership and grants, and
allows the baseline's `public.pg_trgm` extension objects. The bridge leaves
those permissions intact. Sequence counters and every business row also remain
intact. After schema verification, it replaces the ledger with the baseline
marker and applies subsequent migrations in the same transaction. Migration 43
preserves the existing frontier digest while retiring manifest artifacts.

A failed transaction restores the old ledger, schema and business rows. If the
commit response is lost, read the ledger with the pinned candidate binary.
The exact old inventory can retry; the exact new inventory can continue forward
activation. Unknown states leave writers closed. Follow the existing deployment
recovery workflow instead of manually editing ledger rows.

## Restoring a backup

For a backup before migration 42:

1. Restore it into an isolated database with application writers closed.
2. Run `plan` with the pinned original-chain maintenance binary. Perform the
   pre-migration work required by that revision. Backups before migration 33
   also require its original Git segment preparation command and access to the
   referenced storage objects.
3. Run the original binary's maintenance migration and required backfills, then
   require its `verify` command to report the exact 42-migration inventory.
4. Check the restored business data and storage references, take a new backup,
   and run the candidate binary's `plan` and `apply` through the maintenance
   workflow. Verify the new inventory before activating candidate services.

For a backup already at the original migration 42, start with original-binary
verification and the schema/restoration checks, then bridge. For a backup that
already contains the new baseline marker, restore and use only the new
inventory. Keep the original runner available for as long as older backups must
remain restorable. A source SHA by itself is insufficient if its binary and
required storage artifacts cannot be recovered.

## Local verification

The normal PostgreSQL tests cover fresh initialization, retained-row and
sequence preservation, exact-ledger planning, unknown-state rejection, schema
drift, rollback after the old ledger has been deleted, repeated application and
writer-fence enforcement:

```sh
SCOPE_TEST_DATABASE_URL=postgres://scope:scope@127.0.0.1:5432/scope_test \
  CARGO_BUILD_JOBS=2 cargo test -p scope-postgres migration
```

The original-chain rehearsal captured states at 33 and 41 and advanced them to
42 on PostgreSQL 16.15 and 18.6. Every seeded business row and sequence counter
survived. The restored 18.6 schemas matched the baseline catalog, including
parser-normalized CHECK expressions and index predicates. This proof exposed
the historical NOT NULL names handled above.

The 16.15 rehearsal also compared the original 42 schema against the baseline,
restored a populated backup and verified interruption rollback. Its fixtures
included users, authentication identities and repositories. The 18.6 upgrade
fixtures contain a user and authentication identity, with run sequence state
`last_value = 67, is_called = true`. The final candidate maintenance binary
applied the baseline and manifest retirement to both restored 18.6 fixtures.
The checks verified the exact final ledger, unchanged business rows and both
sequence counters, and preserved historical NOT NULL constraint names. A second
application to each fixture passed the same checks as an idempotent no-op.

These local fixtures do not substitute for restoring retained production
backups or validating their external storage objects. Each retained database
still needs its own restoration rehearsal before deployment.
