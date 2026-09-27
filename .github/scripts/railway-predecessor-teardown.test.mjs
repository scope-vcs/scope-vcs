import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { activePredecessors, excludeActivated, recordPredecessors, waitForPredecessors } from './railway-predecessor-teardown.mjs';

const environment = 'staging-id';
function status(activeDeployments) {
  return { environments: { edges: [{ node: { id: environment, serviceInstances: { edges: [{ node: {
    serviceId: 'cache-id', activeDeployments,
  } }] } } }] } };
}

function fixture(t) {
  const directory = mkdtempSync(join(tmpdir(), 'scope-predecessors-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  return directory;
}

function withScope(fn) {
  const previous = [process.env.RAILWAY_PROJECT_ID, process.env.SCOPE_RAILWAY_ENVIRONMENT_ID];
  process.env.RAILWAY_PROJECT_ID = 'project-id';
  process.env.SCOPE_RAILWAY_ENVIRONMENT_ID = environment;
  return Promise.resolve().then(fn).finally(() => {
    for (const [key, value] of [['RAILWAY_PROJECT_ID', previous[0]], ['SCOPE_RAILWAY_ENVIRONMENT_ID', previous[1]]]) {
      if (value === undefined) delete process.env[key]; else process.env[key] = value;
    }
  });
}

test('captures exact active predecessors before mutation, ignoring stale latest deployment', (t) => {
  const directory = fixture(t);
  const ids = activePredecessors(status([{ id: 'old-1' }, { id: 'old-2' }]), environment, 'cache-id');
  recordPredecessors(directory, 'cache', 'cache-id', ids);
  assert.deepEqual(JSON.parse(readFileSync(join(directory, 'cache.json'))),
    { component: 'cache', service: 'cache-id', ids: ['old-1', 'old-2'] });
  assert.throws(() => recordPredecessors(directory, 'cache', 'cache-id', ids), /EEXIST/);
  for (const malformed of [null, {}, status(undefined), status([{}])]) {
    assert.throws(() => activePredecessors(malformed, environment, 'cache-id'));
  }
});

test('resume excludes the exact activated ID without losing other predecessors', (t) => {
  const directory = fixture(t);
  recordPredecessors(directory, 'cache', 'cache-id', ['reused-cache', 'old-cache']);
  excludeActivated(directory, 'cache', 'reused-cache');
  assert.deepEqual(JSON.parse(readFileSync(join(directory, 'cache.json'))).ids, ['old-cache']);
});

test('an idempotent resumed activation does not wait for its own serving ID', async (t) => withScope(async () => {
  const directory = fixture(t);
  recordPredecessors(directory, 'cache', 'cache-id', ['reused-cache']);
  excludeActivated(directory, 'cache', 'reused-cache');
  await waitForPredecessors(directory, { read: () => { throw new Error('Unexpected provider poll'); } });
}));

test('one deadline covers slow teardown across all services', async (t) => withScope(async () => {
  const directory = fixture(t);
  recordPredecessors(directory, 'cache', 'cache-id', ['old-cache']);
  recordPredecessors(directory, 'media-api', 'media-id', ['old-media']);
  let time = 0;
  const calls = [];
  await waitForPredecessors(directory, {
    timeoutMs: 100, intervalMs: 50,
    now: () => time,
    pause: async (delay) => { time += delay; },
    read: (args) => {
      const service = args[args.indexOf('--service') + 1];
      calls.push([service, time]);
      return [{ id: service === 'cache-id' ? 'old-cache' : 'old-media',
        status: time >= (service === 'cache-id' ? 50 : 100) ? 'REMOVED' : 'SUCCESS' }];
    },
  });
  assert.deepEqual(calls, [
    ['cache-id', 0], ['media-id', 0], ['cache-id', 50], ['media-id', 50], ['media-id', 100],
  ]);
}));

test('an unremoved predecessor fails the shared barrier without accepting missing inventory', async (t) => withScope(async () => {
  const directory = fixture(t);
  recordPredecessors(directory, 'cache', 'cache-id', ['old-cache']);
  let time = 0;
  await assert.rejects(waitForPredecessors(directory, {
    timeoutMs: 50, intervalMs: 50,
    now: () => time,
    pause: async (delay) => { time += delay; },
    read: () => [],
  }), /old-cache/);
}));
