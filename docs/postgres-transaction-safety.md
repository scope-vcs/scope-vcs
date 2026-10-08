# PostgreSQL transaction cancellation

Scope pins `sqlx-core` and `sqlx-postgres` to upstream commit
`f1e94ec34dd5f11ad6488a3a8ac7988439a3f757`, which contains
[SQLx's transaction startup repair](https://github.com/transact-rs/sqlx/pull/4394).
The registry release 0.9.0 records transaction depth after awaiting `BEGIN`.
Cancellation during that await leaves its rollback guards with depth zero,
so they do nothing. The pool can then reuse a connection whose server-side
transaction is still open.

Later writes on that connection can return successfully without committing.
A subsequent snapshot can fail to set its isolation level and roll back those
writes. This reproduces the production GitHub-check counter failure: a read
number was returned, rolled back, then rejected when its result was applied.
The database constraint remains unchanged.

The pinned driver records the queued transaction before the await, allowing
drop cleanup to queue its rollback. Core and PostgreSQL must use the same
source because they share SQLx types and TLS interfaces. The pin also includes
intervening upstream changes since 0.9.0; workspace tests and dependency
advisory checks cover the combined dependency update. Git source licenses are
collected from the exact archive verified by `legal/git-sources.json`.

The regression in `db/transaction_cancellation_tests.rs` uses normal pool
acquisition settings and an advisory lock to make cancellation deterministic.
It verifies reuse of the same connection, visibility of a returned check read
from an independent connection, a later metadata snapshot, and applying that
read. It fails against registry 0.9.0 because the returned read is uncommitted.

This repair covers Scope's top-level transaction startup. It does not claim
that cancellation of nested SQLx transactions preserves an outer transaction:
the dependency's nested drop guards require separate upstream work. Scope does
not start nested transactions in its production store paths. Existing store
tests continue to cover ordinary commit, rollback and concurrent check reads.

Replace the Git patches when a published SQLx release includes this repair.
Remove the reviewed Git archive entry when the lockfile no longer references
it, regenerate the license inventory, and rerun the cancellation regression
and backend gate before removing the pin.
