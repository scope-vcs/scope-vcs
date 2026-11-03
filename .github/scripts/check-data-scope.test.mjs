import assert from 'node:assert/strict'
import test from 'node:test'
import { findViolations } from './check-data-scope.mjs'

const lines = (found) => found.map(({ line }) => line)

test('rejects raw transactions outside the data scope owner', () => {
  const source = [
    'use sea_orm::TransactionTrait;',
    'async fn read(db: &sea_orm::DatabaseConnection) {',
    '    let tx = db.begin_with_config(None, None).await?;',
    '    let pool = db.get_postgres_connection_pool();',
    '}',
  ].join('\n')
  assert.deepEqual(lines(findViolations('crates/scope-postgres/src/db/requests.rs', source)), [1, 2, 3, 4])
  assert.deepEqual(lines(findViolations('api/src/persistence.rs', source)), [1, 2, 3, 4])
  assert.deepEqual(findViolations('crates/scope-postgres/src/db/data_scope.rs', source), [])
  assert.deepEqual(findViolations('crates/scope-postgres/src/migrations/mod.rs', source), [])
  assert.deepEqual(findViolations('crates/scope-postgres/src/db/test_support/fixtures.rs', source), [])
})

test('rejects scope settings outside the boundary and its policies', () => {
  const source = "tx.execute_unprepared(\"SELECT set_config('scope.repository_id', 'other', true)\")"
  assert.deepEqual(lines(findViolations('crates/scope-postgres/src/db/outbox.rs', source)), [1])
  assert.deepEqual(lines(findViolations('crates/scope-postgres/src/db/migration_tests.rs', "current_setting('scope.system_reason')")), [1])
  assert.deepEqual(findViolations('crates/scope-postgres/src/db/data_scope.rs', source), [])
  assert.deepEqual(findViolations('crates/scope-postgres/src/migrations/m0107_repository_row_security.rs', source), [])
})

test('allows scoped transactions and non-Rust sources', () => {
  assert.deepEqual(findViolations('crates/scope-postgres/src/db/requests.rs', 'let tx = self.db.begin(DataScope::Repository(repo_id)).await?;'), [])
  assert.deepEqual(findViolations('docs/architecture.md', 'DatabaseConnection'), [])
})
