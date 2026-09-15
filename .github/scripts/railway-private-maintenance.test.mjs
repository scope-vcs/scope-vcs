import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtempSync, writeFileSync, readFileSync, existsSync, rmSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';

const project = '11111111-1111-4111-8111-111111111111';
const production = '22222222-2222-4222-8222-222222222222';
const service = '33333333-3333-4333-8333-333333333333';
const staging = '44444444-4444-4444-8444-444444444444';
const helper = new URL('./railway-private-maintenance.sh', import.meta.url).pathname;
const commandHelper = new URL('./railway-private-command.sh', import.meta.url).pathname;

function fixture(t) {
  const dir = mkdtempSync(join(tmpdir(), 'scope-private-maintenance-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  mkdirSync(join(dir, 'bin'));
  const binary = join(dir, 'maintenance');
  writeFileSync(binary, `#!/bin/sh
set -eu
printf '%s\\n' "$DATABASE_URL" "$SCOPE_MIGRATION_LOCK_TIMEOUT_SECONDS" "$SCOPE_MIGRATION_STATEMENT_TIMEOUT_SECONDS" "$SCOPE_DATA_DIR" "$@" > "$TEST_RESULT"
printf '{"exact":true}\\n'
`);
  const release = { schemaVersion: 1, sourceSha: 'a'.repeat(40), components: {}, maintenanceSha256: createHash('sha256').update(readFileSync(binary)).digest('hex') };
  writeFileSync(join(dir, 'release.json'), JSON.stringify(release));
  writeFileSync(join(dir, 'manifest.json'), JSON.stringify({ railway: { projectId: project, maintenanceServiceId: service }, environments: { production: { environmentId: production }, staging: { environmentId: staging } }, releasePolicy: { migrationLockTimeoutSeconds: 120, migrationStatementTimeoutSeconds: 3600 } }));
  writeFileSync(join(dir, 'bin/railway'), `#!/bin/bash
set -euo pipefail
[[ "$1" == ssh ]]
touch "$TEST_CALLED"
shift
while [[ "$1" != -- ]]; do
  case "$1" in
    --project) export RAILWAY_PROJECT_ID="$2" ;;
    --environment) export RAILWAY_ENVIRONMENT_ID="$2" ;;
    --service) export RAILWAY_SERVICE_ID="$2" ;;
    --identity-file)
      [[ -z "\${SCOPE_RAILWAY_SSH_PRIVATE_KEY:-}" ]]
      [[ "$(stat -c %a "$2")" == 600 ]]
      printf '%s' "$2" > "$TEST_IDENTITY"
      [[ "$(cat "$2")" == 'test-private-key' ]]
      ;;
    *) exit 2 ;;
  esac
  shift 2
done
shift
[[ "$#" == 1 ]]
export RAILWAY_ENVIRONMENT_ID="\${TEST_REMOTE_ENVIRONMENT:-$RAILWAY_ENVIRONMENT_ID}"
export DATABASE_URL='postgres://private.railway.internal/scope'
if [[ "\${TEST_CORRUPT_TRANSFER:-0}" == 1 ]]; then
  printf corrupt | sh -c "$1"
else
  exec sh -c "$1"
fi
`, { mode: 0o700 });
  writeFileSync(join(dir, 'bin/psql'), `#!/bin/sh
set -eu
[ "$1" = 'postgres://private.railway.internal/scope' ]
cat > "$TEST_GRANTS"
exit "\${TEST_GRANT_FAILURE:-0}"
`, { mode: 0o700 });
  const env = { ...process.env, PATH: `${join(dir, 'bin')}:${process.env.PATH}`, SCOPE_DEPLOYMENT_MANIFEST: join(dir, 'manifest.json'), SCOPE_MAINTENANCE_BINARY: binary, SCOPE_PREPARED_RELEASE_PATH: join(dir, 'release.json'), TEST_RESULT: join(dir, 'result'), TEST_CALLED: join(dir, 'called'), TEST_IDENTITY: join(dir, 'identity'), TEST_GRANTS: join(dir, 'grants') };
  delete env.SCOPE_RAILWAY_MAINTENANCE_SERVICE_ID;
  delete env.SCOPE_RAILWAY_SSH_IDENTITY_FILE;
  const run = (args = [production, 'plan'], overrides = {}) => spawnSync('bash', [helper, ...args], { env: { ...env, ...overrides }, encoding: 'utf8' });
  return { dir, env, run, binary };
}

test('runs the digest-bound binary privately and removes transferred files', t => {
  const f = fixture(t);
  for (const environment of [production, staging]) {
    const result = f.run([environment, 'plan']);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(result.stdout), { exact: true });
    const [database, lock, statement, data, command] = readFileSync(f.env.TEST_RESULT, 'utf8').trim().split('\n');
    assert.equal(database, 'postgres://private.railway.internal/scope');
    assert.equal(lock, '120');
    assert.equal(statement, '3600');
    assert.equal(command, 'plan');
    assert.equal(existsSync(join(data, '..')), false);
  }
});

test('rejects unknown targets and command injection before SSH', t => {
  const f = fixture(t);
  for (const args of [[production, 'plan; touch pwned'], ['production', 'plan'], [service, 'plan'], [production, 'plan', 'extra']]) {
    assert.notEqual(f.run(args).status, 0);
    assert.equal(existsSync(f.env.TEST_CALLED), false);
  }
});

test('rejects altered local bytes before SSH', t => {
  const f = fixture(t);
  writeFileSync(f.binary, 'altered');
  const result = f.run();
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /does not match/);
  assert.equal(existsSync(f.env.TEST_CALLED), false);
});

test('rejects corrupted transfer and mismatched remote environment before execution', t => {
  const f = fixture(t);
  for (const overrides of [{ TEST_CORRUPT_TRANSFER: '1' }, { TEST_REMOTE_ENVIRONMENT: staging }]) {
    assert.notEqual(f.run(undefined, overrides).status, 0);
    assert.equal(existsSync(f.env.TEST_RESULT), false);
  }
});

test('private command preserves hostile arguments as literal data', t => {
  const f = fixture(t);
  const attack = `spaces ' quote; $(touch ${join(f.dir, 'pwned')})\nnext line`;
  const result = spawnSync('bash', ['--norc', '-c', 'source "$1"; railway_private_command "$2" printf "%s" "$3"', 'test', commandHelper, production, attack], { env: f.env, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, attack);
  assert.equal(existsSync(join(f.dir, 'pwned')), false);
});

test('temporary SSH identity is private and removed after remote failure', t => {
  const f = fixture(t);
  const result = f.run(undefined, { SCOPE_RAILWAY_SSH_PRIVATE_KEY: 'test-private-key', TEST_REMOTE_ENVIRONMENT: staging });
  assert.notEqual(result.status, 0);
  assert.equal(existsSync(readFileSync(f.env.TEST_IDENTITY, 'utf8')), false);
  assert.doesNotMatch(result.stdout + result.stderr, /test-private-key/);
});

test('apply refreshes runtime grants and surfaces grant failures', t => {
  const f = fixture(t);
  const success = f.run([production, 'apply']);
  assert.equal(success.status, 0, success.stderr);
  assert.match(readFileSync(f.env.TEST_GRANTS, 'utf8'), /GRANT/);
  assert.notEqual(f.run([production, 'apply'], { TEST_GRANT_FAILURE: '1' }).status, 0);
});

test('maintenance image pins PostgreSQL clients and runs only the non-root maintenance server', () => {
  const dockerfile = readFileSync(new URL('../../deploy/railway/maintenance.Dockerfile', import.meta.url), 'utf8');
  assert.match(dockerfile, /^FROM postgres:18\.[0-9]+@sha256:[a-f0-9]{64}$/m);
  assert.match(dockerfile, /^USER 65532:65532$/m);
  assert.match(dockerfile, /^ENTRYPOINT \[\]$/m);
  assert.match(dockerfile, /^CMD \["\/app\/bin\/scope-maintenance", "serve"\]$/m);
});
