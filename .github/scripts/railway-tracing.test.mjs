import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

const manifest = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url), 'utf8'));

function providerFixture(t, confirmed) {
  const directory = mkdtempSync(join(tmpdir(), 'scope-tracing-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const calls = join(directory, 'calls.ndjson');
  writeFileSync(join(directory, 'railway'), `#!/usr/bin/env node
const fs = require('node:fs');
const variables = JSON.parse(fs.readFileSync(0, 'utf8'));
fs.appendFileSync(process.env.TRACING_CALLS, JSON.stringify(variables) + '\\n');
process.stdout.write(JSON.stringify({ data: { serviceInstanceUpdate: ${confirmed} } }));
`, { mode: 0o755 });
  return {
    directory, calls,
    env: { ...process.env, PATH: `${directory}:${process.env.PATH}`, TRACING_CALLS: calls,
      RAILWAY_PROJECT_ID: manifest.railway.projectId,
      SCOPE_RAILWAY_ENVIRONMENT_ID: manifest.environments.production.environmentId,
      RAILWAY_API_TOKEN: 'private-exporter-auth' },
  };
}

test('production configures application services without deployments or database tracing', (t) => {
  const fixture = providerFixture(t, true);
  const result = spawnSync(process.execPath, ['.github/scripts/railway-tracing.mjs', 'production'],
    { env: fixture.env, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  const calls = readFileSync(fixture.calls, 'utf8').trim().split('\n').map(JSON.parse);
  const expectedIds = [...Object.values(manifest.services).map(({ id }) => id), manifest.railway.maintenanceServiceId];
  assert.deepEqual(calls.map(({ serviceId }) => serviceId).sort(), expectedIds.sort());
  for (const { serviceId, environmentId, input } of calls) {
    assert.equal(environmentId, manifest.environments.production.environmentId);
    assert.deepEqual(input, serviceId === manifest.services.web.id
      ? { tracingEnabled: true, autoInstrumentationEnabled: true } : { tracingEnabled: true });
  }
  assert.doesNotMatch(result.stdout + result.stderr, /private-exporter-auth/);
});

test('failed production tracing prevents the monitored release command from activating', (t) => {
  const fixture = providerFixture(t, false);
  const activated = join(fixture.directory, 'activated');
  const result = spawnSync('bash', ['.github/scripts/deploy-monitored-railway.sh', 'backend',
    'touch', activated], { env: fixture.env, encoding: 'utf8', timeout: 15_000 });
  assert.equal(existsSync(fixture.calls), true, 'release must configure tracing before activation');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /release activation blocked/);
  assert.equal(existsSync(activated), false);
  assert.equal(readFileSync(fixture.calls, 'utf8').trim().split('\n').length, 3);
  assert.doesNotMatch(result.stdout + result.stderr, /private-exporter-auth/);
});

test('wrong production target fails before configuring any service', (t) => {
  const fixture = providerFixture(t, true);
  const result = spawnSync(process.execPath, ['.github/scripts/railway-tracing.mjs', 'production'],
    { env: { ...fixture.env, SCOPE_RAILWAY_ENVIRONMENT_ID: manifest.environments.staging.environmentId }, encoding: 'utf8' });
  assert.equal(result.status, 1);
  assert.equal(existsSync(fixture.calls), false);
});
