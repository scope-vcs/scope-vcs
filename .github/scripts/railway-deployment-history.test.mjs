import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

test('deployment inventory failures cannot authorize the no-history branch', () => {
  const run = (response, status = 0) => spawnSync('bash', ['-c', `
    set -euo pipefail
    source .github/scripts/railway-backend-control.sh
    railway_scope=()
    railway_inventory_read() { printf '%s' "$TEST_HISTORY_RESPONSE"; return "$TEST_HISTORY_STATUS"; }
    history="$(service_has_deployment_history service)" || exit $?
    if [[ "$history" == 0 ]]; then echo authorize-bootstrap; else echo require-existing-writer; fi
  `], { encoding: 'utf8', timeout: 15_000, env: {
    ...process.env,
    TEST_HISTORY_RESPONSE: response, TEST_HISTORY_STATUS: String(status),
  } });
  for (const [response, status] of [['', 73], ['[]', 73], ['[{"id":', 0], ['{}', 0], ['null', 0]]) {
    const result = run(response, status);
    assert.notEqual(result.status, 0, `must reject ${response} / status ${status}`);
    assert.equal(result.stdout, '', 'no bootstrap or existing-writer decision before inventory succeeds');
  }
  const empty = run('[]');
  assert.equal(empty.status, 0, empty.stderr || empty.error?.message);
  assert.equal(empty.stdout.trim(), 'authorize-bootstrap');
  const existing = run('[{"id":"existing-deployment"}]');
  assert.equal(existing.status, 0, existing.stderr || existing.error?.message);
  assert.equal(existing.stdout.trim(), 'require-existing-writer');
});
