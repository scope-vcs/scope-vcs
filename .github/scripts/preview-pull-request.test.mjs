import assert from 'node:assert/strict';
import { test } from 'node:test';
import { verifyPreviewBuild } from './preview-pull-request.mjs';

const repository = 'scope-vcs/scope-vcs';
const head = 'a'.repeat(40);
const base = 'b'.repeat(40);
const sourceSha = 'c'.repeat(40);

function scenario({ run = {}, pull = {}, merge, comparison = 'ahead' } = {}) {
  const fullRun = {
    event: 'pull_request', conclusion: 'success', head_sha: head,
    head_repository: { full_name: repository }, pull_requests: [{ number: 7 }], ...run,
  };
  const fullPull = {
    state: 'open', base: { ref: 'main' }, head: { sha: head, repo: { full_name: repository } },
    labels: [{ name: 'preview' }], ...pull,
  };
  const responses = {
    [`/repos/${repository}/pulls/7`]: fullPull,
    [`/repos/${repository}/commits/${sourceSha}`]: merge ?? { parents: [{ sha: base }, { sha: head }] },
    [`/repos/${repository}/compare/${base}...main`]: { status: comparison },
  };
  const requests = [];
  const request = async (path) => {
    requests.push(path);
    if (!(path in responses)) throw new Error(`unexpected ${path}`);
    return responses[path];
  };
  return { requests, verify: () => verifyPreviewBuild({ run: fullRun, repository, sourceSha, request }) };
}

test('deploys the merge build of an open labeled same-repository pull request', async () => {
  assert.deepEqual(await scenario().verify(), { deploy: true, pullRequest: 7, sourceSha });
  assert.deepEqual(await scenario({ comparison: 'identical' }).verify(), { deploy: true, pullRequest: 7, sourceSha });
});

test('skips builds that no longer describe the pull request', async () => {
  for (const options of [
    { run: { conclusion: 'failure' } },
    { run: { event: 'push' } },
    { run: { head_repository: { full_name: 'someone/fork' } } },
    { run: { pull_requests: [] } },
    { pull: { state: 'closed' } },
    { pull: { labels: [] } },
    { pull: { base: { ref: 'release' } } },
    { pull: { head: { sha: 'd'.repeat(40), repo: { full_name: repository } } } },
  ]) {
    const result = await scenario(options).verify();
    assert.equal(result.deploy, false, JSON.stringify(options));
  }
});

test('rejects prepared revisions that are not the pull request merge into main', async () => {
  for (const options of [
    { merge: { parents: [{ sha: base }] } },
    { merge: { parents: [{ sha: base }, { sha: 'd'.repeat(40) }] } },
    { comparison: 'diverged' },
  ]) {
    await assert.rejects(scenario(options).verify(), /not the merge|does not merge/);
  }
});
