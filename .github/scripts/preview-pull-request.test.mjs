import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { mock, test } from 'node:test';
import { buildAction, cleanupAction, reconcileAction } from './preview-pull-request.mjs';

const repository = 'scope-vcs/scope-vcs';
const head = 'a'.repeat(40);
const base = 'b'.repeat(40);
const sourceSha = 'c'.repeat(40);
const manifest = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url)));

function previewInventory(environments) {
  return { id: manifest.railway.projectId, environments: { pageInfo: { hasNextPage: false },
    edges: environments.map((environment, index) => ({ node: {
      id: `00000000-0000-0000-0000-${String(index + 1).padStart(12, '0')}`,
      isEphemeral: true, sourceEnvironment: { id: manifest.environments.staging.environmentId }, ...environment,
    } })),
  } };
}

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
  assert.deepEqual(await scenario({ pull: { base: { ref: 'release' } } }).cleanup(), { action: 'delete', pullRequest: 7 });
  await assert.rejects(scenario().cleanup('7; rm -rf /'), /positive pull request/);
});

test('reconcile finds missed closes and label removals without touching active previews or other environments', async () => {
  const project = previewInventory([
    { name: 'pr-7' }, { name: 'pr-8' }, { name: 'pr-9' }, { name: 'pr-10' },
    { name: 'pr-11', isEphemeral: false },
    { name: 'pr-12', sourceEnvironment: { id: manifest.environments.production.environmentId } },
    { name: 'pr-13', sourceEnvironment: null },
    { name: 'pr-14', id: manifest.environments.production.environmentId },
    { name: 'pr-15', id: manifest.environments.staging.environmentId },
    ...['pr-016', 'pr-0', 'pr-17-extra', 'test-pack-20261110'].map(name => ({ name })),
  ]);
  const pulls = {
    7: openPull({ state: 'closed' }), 8: openPull({ labels: [] }), 9: openPull(),
    10: openPull({ base: { ref: 'release' }, head: { repo: { full_name: 'someone/fork' } } }),
  };
  const request = mock.fn(async path => pulls[path.split('/').at(-1)]);
  assert.deepEqual(await reconcileAction({ manifest, project, repository, request }), {
    pullRequests: [7, 8, 10],
  });
  assert.equal(request.mock.callCount(), 4);
  pulls[7] = openPull();
  pulls[8] = openPull();
  pulls[10] = openPull();
  assert.deepEqual(await cleanupAction({ pullRequest: 7, repository, request }), {
    action: 'skip', reason: 'pull request #7 qualifies for a preview again',
  });
  assert.deepEqual(await reconcileAction({ manifest, project, repository, request }), {
    pullRequests: [],
  });
});

test('reconcile fails closed on incomplete inventory or unconfirmed GitHub state', async () => {
  const request = mock.fn(async () => { throw new Error('GitHub unavailable'); });
  const project = previewInventory([{ name: 'pr-7' }]);
  for (const inventory of [
    { ...project, id: 'another-project' },
    { ...project, environments: { ...project.environments, pageInfo: { hasNextPage: true } } },
    previewInventory([{ name: 'pr-7', id: 'invalid' }]),
    previewInventory([{ name: 'pr-7' }, { name: 'pr-8', id: project.environments.edges[0].node.id }]),
  ]) {
    await assert.rejects(reconcileAction({ manifest, project: inventory, repository, request }), /manifest project|inventory is incomplete|invalid or duplicate/);
  }
  assert.equal(request.mock.callCount(), 0);
  await assert.rejects(reconcileAction({ manifest, project, repository, request }), /GitHub unavailable/);
  for (const pull of [{}, openPull({ labels: null }), openPull({ labels: [null] }), openPull({ state: 'unknown' })]) {
    await assert.rejects(reconcileAction({ manifest, project, repository, request: async () => pull }), /did not confirm/);
  }
});

test('shared cleanup tolerates an environment already deleted by another run and reports actual deletions', () => {
  const workflow = readFileSync(new URL('../workflows/preview-environment-cleanup.yml', import.meta.url), 'utf8');
  const script = workflow.split('      - name: Delete the pull request environment\n')[1]
    .split('        run: |\n')[1].split('\n      - name:')[0].replace(/^          /gm, '');
  const dir = mkdtempSync(join(tmpdir(), 'preview-cleanup-'));
  try {
    writeFileSync(join(dir, 'node'), '#!/usr/bin/env bash\nprintf \'%s\\n\' "$DELETE_RESULT"\n', { mode: 0o755 });
    for (const deleted of [false, true]) {
      const result = { deleted, name: 'pr-7', ...(deleted && { environmentId: 'preview-id' }) };
      const output = join(dir, `output-${deleted}`);
      const summary = join(dir, `summary-${deleted}`);
      writeFileSync(summary, '');
      const run = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', script], {
        encoding: 'utf8', env: { ...process.env, PATH: `${dir}:${process.env.PATH}`,
          DELETE_RESULT: JSON.stringify(result), PULL_REQUEST: '7', GITHUB_OUTPUT: output, GITHUB_STEP_SUMMARY: summary },
      });
      assert.equal(run.status, 0, run.stderr);
      assert.match(readFileSync(output, 'utf8'), new RegExp(`deleted=${deleted}\\n`));
      assert.equal(readFileSync(summary, 'utf8'), deleted ? 'Deleted preview environment `pr-7` (`preview-id`) for PR #7.\n' : '');
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('Railway inventory reader preserves complete environment identities for reconcile and audit', () => {
  const dir = mkdtempSync(join(tmpdir(), 'preview-inventory-'));
  try {
    const response = JSON.stringify({ data: { project: previewInventory([{ name: 'pr-7' }]) } });
    const requestPath = join(dir, 'request');
    writeFileSync(join(dir, 'curl'), '#!/usr/bin/env bash\nprintf \'%s\\n\' "${@: -1}" > "$INVENTORY_REQUEST"\nprintf \'%s\\n\' "$INVENTORY_RESPONSE"\n', { mode: 0o755 });
    const result = execFileSync('bash', ['.github/scripts/read-railway-environments.sh'], {
      encoding: 'utf8', env: { ...process.env, PATH: `${dir}:${process.env.PATH}`, RAILWAY_API_TOKEN: 'fixture',
        INVENTORY_REQUEST: requestPath, INVENTORY_RESPONSE: response },
    });
    assert.equal(result.trim(), response);
    const request = JSON.parse(readFileSync(requestPath, 'utf8'));
    assert.deepEqual(request.variables, { id: manifest.railway.projectId });
    assert.match(request.query, /pageInfo\{hasNextPage\}/);
    assert.match(request.query, /isEphemeral sourceEnvironment\{id\}/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('scheduled and event cleanup serialize with deploys, recheck before deleting, and audit after failed or skipped cleanup', () => {
  const workflows = JSON.parse(execFileSync('python3', ['-c',
    "import json, pathlib, yaml; print(json.dumps({name: yaml.safe_load(pathlib.Path('.github/workflows/' + name + '.yml').read_text())['jobs'] for name in ['preview-environment', 'preview-environment-cleanup', 'audit-railway-experiments']}))",
  ], { encoding: 'utf8' }));
  const event = workflows['preview-environment'];
  const scheduled = workflows['audit-railway-experiments'];
  const shared = workflows['preview-environment-cleanup'].delete;
  assert.equal(event.delete.uses, scheduled.cleanup.uses);
  assert.equal(shared.concurrency.group.replace('inputs.pull_request', 'needs.plan.outputs.pull_request'), event.deploy.concurrency.group);
  assert.equal(shared.concurrency['cancel-in-progress'], false);
  const recheck = shared.steps.findIndex(step => step.id === 'recheck');
  const deletion = shared.steps.findIndex(step => step.id === 'delete');
  assert.equal(deletion, recheck + 1);
  assert.match(shared.steps[recheck].run, /preview-pull-request\.mjs cleanup/);
  assert.equal(shared.steps[deletion].if, "steps.recheck.outputs.action == 'delete'");
  assert.equal(scheduled.cleanup.strategy['fail-fast'], false);
  assert.ok(scheduled.audit.needs.includes('cleanup'));
  const auditCondition = Function('cancelled', 'github', `return ${scheduled.audit.if.slice(3, -2)};`);
  assert.equal(auditCondition(() => false, { ref: 'refs/heads/main' }), true);
  assert.equal(auditCondition(() => true, { ref: 'refs/heads/main' }), false);
  assert.ok(scheduled.audit.steps.some(step => step.run?.includes('read-railway-environments.sh')));
  assert.match(scheduled.audit.steps.find(step => step.name === 'Read open pull requests').run, /state, labels/);
});
