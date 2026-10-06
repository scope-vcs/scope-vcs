import assert from 'node:assert/strict';
import { test } from 'node:test';
import { buildAction, cleanupAction } from './preview-pull-request.mjs';

const repository = 'scope-vcs/scope-vcs';
const head = 'a'.repeat(40);
const base = 'b'.repeat(40);
const sourceSha = 'c'.repeat(40);

function openPull(overrides = {}) {
  return {
    state: 'open', base: { ref: 'main' }, head: { sha: head, repo: { full_name: repository } },
    labels: [{ name: 'preview' }], ...overrides,
  };
}

function scenario({ run = {}, pull = {}, merge, comparison = 'ahead', prepared = sourceSha } = {}) {
  const fullRun = {
    event: 'pull_request', conclusion: 'success', head_sha: head,
    head_repository: { full_name: repository }, pull_requests: [{ number: 7 }], ...run,
  };
  const responses = {
    [`/repos/${repository}/pulls/7`]: openPull(pull),
    [`/repos/${repository}/commits/${sourceSha}`]: merge ?? { parents: [{ sha: base }, { sha: head }] },
    [`/repos/${repository}/compare/${base}...main`]: { status: comparison },
  };
  const request = async (path) => {
    if (!(path in responses)) throw new Error(`unexpected ${path}`);
    return responses[path];
  };
  return {
    build: () => buildAction({ run: fullRun, repository, sourceSha: prepared, request }),
    cleanup: (pullRequest = 7) => cleanupAction({ pullRequest, repository, request }),
  };
}

test('deploys the merge build of an open labeled same-repository pull request', async () => {
  assert.deepEqual(await scenario().build(), { action: 'deploy', pullRequest: 7, sourceSha });
  assert.deepEqual(await scenario({ comparison: 'identical' }).build(), { action: 'deploy', pullRequest: 7, sourceSha });
});

test('skips runs that built nothing or no longer describe the pull request head', async () => {
  for (const options of [
    { prepared: '' },
    { run: { conclusion: 'failure' } },
    { run: { event: 'push' } },
    { run: { head_repository: { full_name: 'someone/fork' } } },
    { run: { pull_requests: [] } },
    { pull: { head: { sha: 'd'.repeat(40), repo: { full_name: repository } } } },
  ]) {
    assert.equal((await scenario(options).build()).action, 'skip', JSON.stringify(options));
  }
});

test('a finished build deletes the preview of a pull request that stopped qualifying', async () => {
  for (const pull of [{ state: 'closed' }, { labels: [] }, { base: { ref: 'release' } }]) {
    assert.deepEqual(await scenario({ pull }).build(), {
      action: 'delete', pullRequest: 7, reason: 'pull request #7 no longer qualifies for a preview',
    });
  }
});

test('rejects prepared revisions that are not the pull request merge into main', async () => {
  for (const options of [
    { merge: { parents: [{ sha: base }] } },
    { merge: { parents: [{ sha: base }, { sha: 'd'.repeat(40) }] } },
    { comparison: 'diverged' },
  ]) {
    await assert.rejects(scenario(options).build(), /not the merge|does not merge/);
  }
});

test('cleanup deletes only when the pull request still does not qualify', async () => {
  assert.deepEqual(await scenario({ pull: { state: 'closed' } }).cleanup(), { action: 'delete', pullRequest: 7 });
  assert.deepEqual(await scenario({ pull: { labels: [] } }).cleanup(), { action: 'delete', pullRequest: 7 });
  assert.equal((await scenario().cleanup()).action, 'skip');
  await assert.rejects(scenario().cleanup('7; rm -rf /'), /positive pull request/);
});
