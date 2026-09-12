# Current schema baseline and retained databases

The active inventory starts at `m0042_current_schema_baseline`, followed by
`m0043_retire_git_manifests`. A fresh database creates the current schema
directly. A retained database uses a one-time maintenance bridge after reaching
the exact original inventory through `m0042_request_media`.

The original-chain source is pinned at
`578bec00da088598919082b35a7153f62bf0b860`. Keep its maintenance binary, source
revision and required storage configuration with every backup that predates the
baseline. The old chain is not compiled into the current application.

## Migration definitions are immutable

`dev/checks/policy` verifies the SHA-256 digests in
`crates/scope-postgres/src/migrations/sources.lock.json`. The frozen baseline
SQL, original ledger, and numbered migration sources must retain their bytes.
Pull request checks also compare existing lock entries with the target branch,
so updating a checksum alongside an old migration is rejected.

Change an existing database with a new numbered migration and add its digest
to the lock. Keep migration SQL self-contained; calling mutable application
helpers can change historical migration behavior without changing its source.
Do not edit existing migrations or regenerate their lock entries to make a
schema check pass.

## Preparing a retained database

Before deployment, settle which databases and backups must be retained,
preserve their restore artifacts, and rehearse their transitions. A database
behind the original migration 42 first advances to it with the pinned original
runner; a database at 42 needs schema and restoration proof before bridging.
Preserve immutable images referenced by unresolved cutovers as described in
[maintenance migration recovery](../maintenance-cutovers.md).

## Maintenance behavior

`scope-maintenance plan` reports the baseline as maintenance required when the
ledger contains exactly the original 42 migrations. Ordinary startup refuses
that plan. Older, incomplete, unknown and newer original histories are rejected
with the pinned source revision in the diagnostic. The current binary never
fills gaps or resets a database to make its ledger fit.

`scope-maintenance preflight` checks the planned baseline transition against
the actual schema before deployment closes production writers. It uses the
same schema comparison as the bridge in a transaction that always rolls back
its comparison metadata. Release preparation, direct backend deployment, and
staging baseline adoption run this check. `plan` remains a ledger-only operation
for recovery after a migration has committed. Preflight validates the original
chain, a baseline awaiting migration 43, and empty-schema initialization; it does not replay data migrations
or claim to validate every schema object after later migrations.

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

Push intents now encode `base_git_frontier` directly in version 2 claims.
Version 1 tokens are rejected; a push prepared before the cutover must obtain
a fresh intent before retrying.

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
   and run the candidate binary's `preflight` and `apply` through the maintenance
   workflow. Verify the new inventory before activating candidate services.

For a backup already at the original migration 42, start with original-binary
verification and the schema/restoration checks, then bridge. For a backup that
already contains the new baseline marker, restore and use only the new
inventory. Keep the original runner available for as long as older backups must
remain restorable. A source SHA by itself is insufficient if its binary and
required storage artifacts cannot be recovered.

An exact original ledger does not prove that the schema is correct. Rerunning
the original binary skips migrations already recorded as applied, including
any whose source was subsequently edited. A schema mismatch must be diagnosed
and repaired explicitly before bridging; never stamp or reset ledger entries.

## September 10, 2026 production constraint repair

Commit `7b831951` changed `m0033_git_segment_streaming_v2` after production had
applied it. It added `retained` to the allowed upload states and to the states
requiring complete upload metadata. Production kept the two earlier CHECK
definitions. Staging had the newer definitions under the same migration ledger.
The later baseline captured the newer schema.

The release's production inspection and staging baseline comparison used only
migration names. The upgrade fixture also installed the candidate baseline and
stamped historical names, so both checks missed the physical difference.
Production's strict bridge correctly rejected the two constraints, but only
after maintenance had started.

The approved repair replaced only `scope_git_segment_upload_state` and
`scope_git_segment_upload_values` on `scope_git_segment_uploads` with the
baseline's validated CHECK definitions. A transaction checked the exact
42-migration ledger, required that these were the only schema differences,
verified complete schema equality afterward, and verified unchanged upload
rows. The same transaction was rehearsed with rollback before it was committed.
No migration names or business rows were rewritten.

Backups taken before that repair may retain the older CHECKs. Preserve this
diagnosis with those backups and compare their actual schema before recovery.
The original runner cannot repair an already-applied migration by running again.
This is distinct from the earlier index comparison error, where PostgreSQL
qualified `public.gin_trgm_ops` differently because of the comparison search path.

## Local verification

The normal PostgreSQL tests cover fresh initialization, retained-row and
sequence preservation, exact-ledger planning, unknown-state rejection, schema
drift, rollback after the old ledger has been deleted, repeated application and
writer-fence enforcement:

Baseline bridge tests load a frozen schema-only snapshot in
`crates/scope-postgres/src/db/migration_tests/fixtures/original_chain_schema.sql`
instead of creating the schema through the candidate migration. Preflight tests
also reproduce the earlier upload CHECK definitions and require rejection with
unchanged rows and ledger while application writers remain open.

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
