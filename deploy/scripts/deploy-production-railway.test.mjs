import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

const manifest = JSON.parse(readFileSync(new URL('../../.github/deployment-services.json', import.meta.url), 'utf8'));
const production = manifest.environments.production.environmentId;
const sourceSha = 'a'.repeat(40);

function release(t, components, { maintenance = true, selected = components, env = {} } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'scope-production-deploy-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, 'deploy/scripts'), { recursive: true });
  mkdirSync(join(root, '.github'));
  writeFileSync(join(root, '.github/deployment-services.json'), JSON.stringify(manifest));
  writeFileSync(join(root, 'prepared.json'), JSON.stringify({
    ...(maintenance ? { maintenanceSha256: 'b'.repeat(64) } : {}),
    components: Object.fromEntries(components.map((component) => [component, { image: `image-of-${component}` }])),
  }));
  const record = 'import { appendFileSync } from "node:fs"; appendFileSync("events", `${process.env.SCOPE_DEPLOYMENT_COMPONENT ?? ""}|${process.argv.slice(1).join(" ").replace(/^.*deploy\\/scripts\\//, "")}\\n`);';
  for (const script of ['railway-artifact.mjs', 'railway-predecessor-teardown.mjs']) writeFileSync(join(root, 'deploy/scripts', script), record);
  writeFileSync(join(root, 'deploy/scripts/deploy-railway-image.mjs'), `${record}
    if (process.argv[2] !== "configure-registry") appendFileSync(process.env.SCOPE_DEPLOYMENT_EVIDENCE_PATH, JSON.stringify({ component: process.env.SCOPE_DEPLOYMENT_COMPONENT, evidenceId: "deployment-of-" + process.env.SCOPE_DEPLOYMENT_COMPONENT }) + "\\n");`);
  writeFileSync(join(root, 'deploy/scripts/railway-service-health.sh'), `
    railway_config_path() { printf 'config-of-%s\\n' "$1"; }
    wait_for_service_health() {
      printf '%s|health %s\\n' "$SCOPE_DEPLOYMENT_COMPONENT" "$*" >> events
      [[ -z "\${UNHEALTHY:-}" && ( -z "\${CRASHES_DURING_TEARDOWN:-}" || "$(grep -c "^$SCOPE_DEPLOYMENT_COMPONENT|health " events)" == 1 ) ]]
    }
  `);
  for (const script of ['railway-private-maintenance.sh', 'deploy-railway.sh']) {
    writeFileSync(join(root, 'deploy/scripts', script), `printf '%s|${script} %s\\n' "\${SCOPE_DEPLOYMENT_COMPONENT:-}" "$*" >> events\n`);
  }
  const result = spawnSync('bash', [new URL('./deploy-production-railway.sh', import.meta.url).pathname], {
    cwd: root, encoding: 'utf8',
    env: {
      PATH: process.env.PATH, RAILWAY_TOKEN: 'production-token', SCOPE_PREPARED_RELEASE_PATH: 'prepared.json',
      SCOPE_DEPLOYMENT_SOURCE_SHA: sourceSha, SCOPE_DEPLOYMENT_EVIDENCE_PATH: 'evidence.ndjson',
      SCOPE_MAINTENANCE_BINARY: 'scope-maintenance', SCOPE_RELEASE_COMPONENTS: JSON.stringify(selected), ...env,
    },
  });
  let events = [];
  try { events = readFileSync(join(root, 'events'), 'utf8').trim().split('\n'); } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }
  let evidence = [];
  try { evidence = readFileSync(join(root, 'evidence.ndjson'), 'utf8').trim().split('\n').map((line) => JSON.parse(line).component); } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }
  return { ...result, events, evidence };
}

const applications = Object.keys(manifest.services).filter((component) => manifest.services[component].deployment.artifact.kind !== 'external-image');

test('a backend release migrates before activating each prepared service after the services it depends on', (t) => {
  const prepared = Object.keys(manifest.services);
  const result = release(t, [...prepared].reverse());
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.events.slice(0, 4), [
    `|railway-artifact.mjs validate prepared.json ${sourceSha} ${prepared.join(' ')}`,
    `|railway-private-maintenance.sh ${production} plan`,
    `|railway-private-maintenance.sh ${production} apply`,
    `|railway-private-maintenance.sh ${production} verify`,
  ]);
  const activated = [...new Set(result.events.slice(4).map((event) => event.split('|')[0]))];
  assert.deepEqual(activated, prepared);
  for (const [index, component] of activated.entries()) {
    for (const dependency of manifest.services[component].dependsOn) {
      if (activated.includes(dependency)) assert.ok(activated.indexOf(dependency) < index, `${component} follows ${dependency}`);
    }
  }
  for (const component of prepared) {
    const service = manifest.services[component].id;
    const health = `${component}|health ${service} deployment-of-${component} config-of-${component}`;
    const expected = applications.includes(component)
      ? [`${component}|deploy-railway.sh ${service}`]
      : [`${component}|deploy-railway-image.mjs configure-registry ${service}`, `${component}|deploy-railway-image.mjs ${service} image-of-${component}`,
        health, `${component}|railway-predecessor-teardown.mjs wait`, health];
    assert.deepEqual(result.events.filter((event) => event.startsWith(`${component}|`)).map((event) => event.replace(/( wait) .*$/, '$1')), expected);
  }
});

test('an image service that is unhealthy before or after its predecessor leaves is not recorded and stops the release', (t) => {
  const prepared = Object.keys(manifest.services);
  const images = prepared.filter((component) => !applications.includes(component));
  assert.deepEqual(release(t, prepared).evidence, images);
  const after = prepared.slice(prepared.indexOf(images[0]) + 1);
  assert.ok(after.length > 0);
  for (const failure of ['UNHEALTHY', 'CRASHES_DURING_TEARDOWN']) {
    const stopped = release(t, prepared, { env: { [failure]: '1' } });
    assert.notEqual(stopped.status, 0, failure);
    assert.deepEqual(stopped.evidence, [], failure);
    assert.ok(after.every((component) => !stopped.events.some((event) => event.startsWith(`${component}|`))), failure);
  }
});

test('a release activates only the services its plan selected, and still migrates', (t) => {
  const prepared = Object.keys(manifest.services);
  const selected = prepared.filter((component) => manifest.services[component].dependsOn.length > 0).slice(-2);
  const result = release(t, prepared, { selected: [...selected].reverse() });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.events[0], `|railway-artifact.mjs validate prepared.json ${sourceSha} ${selected.join(' ')}`);
  assert.deepEqual(result.events.slice(1, 4).map((event) => event.split(' ').at(-1)), ['plan', 'apply', 'verify']);
  assert.deepEqual([...new Set(result.events.slice(4).map((event) => event.split('|')[0]))], selected);
});

test('a release without backend images activates without touching the database', (t) => {
  const result = release(t, ['web'], { maintenance: false, env: { SCOPE_MAINTENANCE_BINARY: '' } });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.events, [
    `|railway-artifact.mjs validate prepared.json ${sourceSha} web`,
    `web|deploy-railway.sh ${manifest.services.web.id}`,
  ]);
});

test('production refuses account tokens, preview targets and empty releases before any change', (t) => {
  for (const env of [
    { RAILWAY_API_TOKEN: 'account-token' },
    { RAILWAY_TOKEN: '' },
    { SCOPE_RAILWAY_PREVIEW_ENVIRONMENT_ID: production },
    { SCOPE_ROLE_GRANTS_SQL: 'grants.sql' },
    { SCOPE_DEPLOYMENT_SOURCE_SHA: 'abc123' },
    { SCOPE_MAINTENANCE_BINARY: '' },
    { SCOPE_RELEASE_COMPONENTS: '' },
    { SCOPE_RELEASE_COMPONENTS: '[]' },
  ]) {
    const result = release(t, ['api', 'web'], { env });
    assert.notEqual(result.status, 0, JSON.stringify(env));
    assert.deepEqual(result.events.filter((event) => !event.includes('validate')), [], JSON.stringify(env));
  }
  const empty = release(t, []);
  assert.notEqual(empty.status, 0);
  assert.deepEqual(empty.events, []);
});
