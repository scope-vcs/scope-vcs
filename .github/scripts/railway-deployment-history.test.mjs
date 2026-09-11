import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

test('deployment inventory failures cannot authorize the no-history branch', t => {
  const root = mkdtempSync(join(tmpdir(), 'scope-deployment-history-'));
  t.after(() => rmSync(root, { force: true, recursive: true }));
  writeFileSync(join(root, 'railway'), '#!/bin/sh\nprintf "%s" "$TEST_HISTORY_RESPONSE"\nexit "$TEST_HISTORY_STATUS"\n', { mode: 0o755 });
  const run = (response, status = 0) => spawnSync('bash', ['-c', `
    set -euo pipefail
    source .github/scripts/railway-backend-control.sh
    railway_scope=()
    history="$(service_has_deployment_history service)" || exit $?
    if [[ "$history" == 0 ]]; then echo authorize-bootstrap; else echo require-existing-writer; fi
  `], { encoding: 'utf8', timeout: 15_000, env: {
    ...process.env, PATH: `${root}:${process.env.PATH}`,
    TEST_HISTORY_RESPONSE: response, TEST_HISTORY_STATUS: String(status),
  } });
  for (const [response, status] of [['', 73], ['[]', 73], ['[{"id":', 0], ['{}', 0], ['null', 0]]) {
    const result = run(response, status);
    assert.notEqual(result.status, 0, `must reject ${response} / status ${status}`);
    assert.equal(result.stdout, '', 'no bootstrap or existing-writer decision before inventory succeeds');
  }
  assert.equal(run('[]').stdout.trim(), 'authorize-bootstrap');
  assert.equal(run('[{"id":"existing-deployment"}]').stdout.trim(), 'require-existing-writer');
});
