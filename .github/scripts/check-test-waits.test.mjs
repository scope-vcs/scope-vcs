import assert from 'node:assert/strict'
import test from 'node:test'
import { findLongWaits, testLanguage } from './check-test-waits.mjs'

const lines = (found) => found.map(({ line }) => line)

test('classifies test sources by path', () => {
  assert.equal(testLanguage('cli/tests/push.rs'), 'rust')
  assert.equal(testLanguage('api/src/git/cache/tests.rs'), 'rust')
  assert.equal(testLanguage('crates/scope-postgres/src/db/repo_collaboration_tests.rs'), 'rust')
  assert.equal(testLanguage('api/src/workflow_tests/auth.rs'), 'rust')
  assert.equal(testLanguage('crates/scope-postgres/src/db/test_support/database_templates.rs'), 'rust')
  assert.equal(testLanguage('api/src/background_work.rs'), 'rust-inline')
  assert.equal(testLanguage('web/src/analytics/delivery.test.ts'), 'script')
  assert.equal(testLanguage('web/smoke/skeleton-layout.spec.mjs'), 'script')
  assert.equal(testLanguage('web/smoke/browser-smoke.mjs'), 'script')
  assert.equal(testLanguage('web/src/analytics/delivery.ts'), null)
  assert.equal(testLanguage('dev/test_local_stack.py'), 'python')
  assert.equal(testLanguage('.github/scripts/test-deploy-backend-railway.sh'), 'shell')
  assert.equal(testLanguage('.github/scripts/backend-cutover-test-provider.sh'), 'shell')
  assert.equal(testLanguage('.github/scripts/deploy-railway.sh'), null)
})

test('rejects long real-clock sleeps in Rust tests', () => {
  const source = [
    '#[tokio::test]',
    'async fn waits() {',
    '    tokio::time::sleep(std::time::Duration::from_secs(61)).await;',
    '    tokio::time::sleep(Duration::from_millis(5_000)).await;',
    '    std::thread::sleep(Duration::from_millis(4_999));',
    '    sleep(Duration::from_secs_f64(0.5)).await;',
    '}',
  ].join('\n')
  assert.deepEqual(lines(findLongWaits('api/src/merge/tests.rs', source)), [3, 4])
})

test('rejects computed second-scale sleeps split across lines', () => {
  const source = [
    '#[tokio::test]',
    'async fn expires() {',
    '    tokio::time::sleep(std::time::Duration::from_secs(',
    '        GITHUB_CHECK_MERGE_FRESHNESS_SECS + 1,',
    '    ))',
    '    .await;',
    '    tokio::time::sleep(LOCK_RETRY).await;',
    '    tokio::time::sleep(Duration::from_millis(delay as u64)).await;',
    '}',
  ].join('\n')
  assert.deepEqual(lines(findLongWaits('crates/scope-postgres/src/db/request_merge/tests.rs', source)), [3])
})

test('allows long sleeps on a paused tokio clock', () => {
  const source = [
    '#[tokio::test(flavor = "current_thread", start_paused = true)]',
    'async fn paused() {',
    '    tokio::time::sleep(Duration::from_secs(30)).await;',
    '}',
    '#[tokio::test]',
    'async fn paused_inside() {',
    '    tokio::time::pause();',
    '    tokio::time::sleep(Duration::from_secs(30)).await;',
    '}',
    '#[tokio::test]',
    'async fn real() {',
    '    tokio::time::sleep(Duration::from_secs(30)).await;',
    '}',
  ].join('\n')
  assert.deepEqual(lines(findLongWaits('crates/x/tests/clock.rs', source)), [12])
})

test('checks only inline test modules in production Rust files', () => {
  const source = [
    'fn retry() {',
    '    thread::sleep(Duration::from_secs(10));',
    '}',
    '#[cfg(test)]',
    'mod tests {',
    '    fn waits() { thread::sleep(Duration::from_secs(10)); }',
    '}',
  ].join('\n')
  assert.deepEqual(lines(findLongWaits('cli/src/push.rs', source)), [6])
  const external = 'fn retry() { thread::sleep(Duration::from_secs(10)); }\n#[cfg(test)]\nmod tests;'
  assert.deepEqual(findLongWaits('cli/src/push.rs', external), [])
})

const call = (callee, args) => `await ${callee}(${args})`

test('rejects long fixed waits in script tests but not timeout guards', () => {
  const source = [
    call('page.waitForTimeout', '5_000'),
    call('new Promise((resolve) => setTimeout', 'resolve, 6000)'),
    call('delay', '10_000'),
    call('setTimeout', '5000'),
    call('setTimeout', '() => reject(new Error("stuck")), 30_000'),
    call('page.waitForTimeout', '700'),
  ].join('\n')
  assert.deepEqual(lines(findLongWaits('web/smoke/layout.spec.mjs', source)), [1, 2, 3, 4])
  assert.deepEqual(findLongWaits('web/src/lib/delay.ts', call('delay', '10_000')), [])
})

test('rejects long sleeps in Python and shell test harnesses', () => {
  assert.deepEqual(lines(findLongWaits('dev/test_stack.py', 'time.sleep(5)\ntime.sleep(0.5)')), [1])
  const shell = 'sleep 0.05\nretry || sleep 5\n/bin/sleep 12\n'
  assert.deepEqual(lines(findLongWaits('.github/scripts/test-deploy.sh', shell)), [2, 3])
})
