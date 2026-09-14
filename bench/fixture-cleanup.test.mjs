import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { mkdir, mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { FixtureCleanup } from './fixture-cleanup.mjs';
import { createFetchClients, seedRepository } from './railway-load.mjs';
import { execute } from './subprocess.mjs';

async function setup(t, reply) {
  const root = await mkdtemp(join(tmpdir(), 'scope-fixtures-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const server = createServer(reply);
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  t.after(() => { server.closeAllConnections(); server.close(); });
  const origin = `http://127.0.0.1:${server.address().port}`;
  const config = { token: 'test', timeoutMs: 1000, endpointRouter: { choose: () => origin } };
  return { root, origin, config };
}

function repositoryResponse(name) {
  return { repo: { owner_handle: 'owner', name }, init: { git_remote_url: 'http://localhost/git/private', token: { secret: 'test' } } };
}

test('remote creation remains owned when local directory allocation fails', async (t) => {
  const { root, config } = await setup(t, (_req, res) => res.end(JSON.stringify(repositoryResponse('created'))));
  const deleted = [];
  const cleanup = new FixtureCleanup(async (fixture) => deleted.push(fixture.repo));
  await assert.rejects(seedRepository(config, cleanup, join(root, 'missing'), 'test', 1, 1), /ENOENT/);
  const result = await cleanup.run();
  assert.deepEqual(deleted, ['created']);
  assert.equal(result.attemptedRepositories, 1);
  assert.equal(result.attemptedDirectories, 0);
});

test('failed retry attempts retain every remote and directory and report deletion failures', async (t) => {
  let created = 0;
  const { root, config } = await setup(t, (req, res) => {
    if (req.method === 'POST' && req.url === '/v1/repos') res.end(JSON.stringify(repositoryResponse(`attempt-${++created}`)));
    else { res.statusCode = 503; res.end('temporary failure'); }
  });
  const deleted = [];
  const cleanup = new FixtureCleanup(async (fixture) => {
    deleted.push(fixture.repo);
    if (fixture.repo === 'attempt-1') throw new Error('delete unavailable');
  });
  await assert.rejects(seedRepository(config, cleanup, root, 'test', 1, 1), /HTTP 503/);
  const result = await cleanup.run();
  assert.equal(created, 3);
  assert.equal(result.attemptedRepositories, 3);
  assert.equal(result.attemptedDirectories, 3);
  assert.deepEqual(deleted.sort(), ['attempt-1', 'attempt-2', 'attempt-3']);
  assert.deepEqual(await readdir(root), []);
  assert.deepEqual(result.failed, [{ repo: 'owner/attempt-1', error: 'delete unavailable' }]);
});

test('a later fetch clone failure retains successful and failing client allocations', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'scope-fetch-clients-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const source = join(root, 'source.git');
  assert.equal((await execute('git', ['init', '--bare', source])).code, 0);
  const clientsRoot = join(root, 'clients');
  await mkdir(clientsRoot);
  const cleanup = new FixtureCleanup(async () => {});
  const config = { repositoryMode: 'spread', mixedRepos: 2, stages: [1], timeoutMs: 2000,
    endpointRouter: { choose: () => 'file:///' } };
  const fixtures = [source, join(root, 'missing.git')].map((path, index) => ({
    owner: 'owner', repo: `repo-${index}`, publicRemotePath: path,
  }));
  await assert.rejects(createFetchClients(config, cleanup, clientsRoot, fixtures), /repository|read from remote/i);
  const result = await cleanup.run();
  assert.equal(result.attemptedClients, 2);
  assert.deepEqual(result.failed, []);
  assert.deepEqual(await readdir(clientsRoot), []);
});
