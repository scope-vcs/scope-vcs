import assert from 'node:assert/strict';
import { mock, test } from 'node:test';
import { advisoryIssueBody, checkMainAdvisories, reportMainAdvisories } from './main-advisories.mjs';

const runUrl = 'https://github.com/scope-vcs/scope-vcs/actions/runs/123';
const sha = 'a'.repeat(40);
const diagnostic = (severity, id, name) => JSON.stringify({type: 'diagnostic', fields: {
  severity, code: 'vulnerability', message: 'security vulnerability detected',
  notes: [`ID: ${id}`], graphs: [{Krate: {name, version: '1.2.3'}, parents: [{Krate: {name: 'parent', version: '1.0.0'}}]}],
}});
const checks = [{target: 'workspace', status: 1, output: [
  diagnostic('error', 'RUSTSEC-2026-0001', 'affected'),
  diagnostic('warning', 'RUSTSEC-2026-0002', 'warning-only'),
  diagnostic('note', 'RUSTSEC-2026-0003', 'ignored'),
].join('\n')}, {target: 'cli', status: 1, output: diagnostic('error', 'RUSTSEC-2026-0004', 'cli-dependency')}];

test('both checks run after an advisory failure or a process launch failure', () => {
  for (const failure of [{status: 1, stderr: 'advisory failure'}, {status: null, error: new Error('spawn failed')}]) {
    const run = mock.fn(() => failure);
    assert.deepEqual(checkMainAdvisories(run).map(check => check.status), [failure.status, failure.status]);
    assert.equal(run.mock.callCount(), 2);
  }
});

test('the issue identifies denied advisories and affected crates in both lockfiles with the main commit and run', () => {
  const body = advisoryIssueBody({checks, runUrl, sha});
  assert.match(body, /RUSTSEC-2026-0001: `affected 1.2.3`/);
  assert.match(body, /RUSTSEC-2026-0004: `cli-dependency 1.2.3`/);
  assert.doesNotMatch(body, /warning-only|ignored|parent/);
  assert.ok(body.includes(runUrl));
  assert.ok(body.includes(sha));
  const setup = advisoryIssueBody({checks: [], runUrl, sha});
  assert.match(setup, /setup failure/);
  const failed = advisoryIssueBody({checks: [{target: 'cli', status: null, output: '', error: 'spawn failed'}], runUrl, sha});
  assert.match(failed, /No advisory IDs.*\n[\s\S]*spawn failed/);
});

test('recurring failures comment on the existing open issue instead of opening duplicates', async () => {
  const open = [{number: 8, title: 'An unrelated bug'}];
  const create = mock.fn(async issue => { open.push({number: 9, ...issue}); });
  const createComment = mock.fn(async () => {});
  const listForRepo = mock.fn();
  const github = {paginate: mock.fn(async () => open), rest: {issues: {listForRepo, create, createComment}}};
  const args = {github, owner: 'scope-vcs', repo: 'scope-vcs', checks, runUrl, sha};
  await reportMainAdvisories(args);
  await reportMainAdvisories(args);
  assert.equal(create.mock.callCount(), 1);
  assert.equal(createComment.mock.callCount(), 1);
  assert.equal(create.mock.calls[0].arguments[0].title, 'New dependency advisory on main');
  assert.deepEqual(create.mock.calls[0].arguments[0].labels, ['bug']);
  assert.equal(createComment.mock.calls[0].arguments[0].issue_number, 9);
  assert.match(createComment.mock.calls[0].arguments[0].body, /RUSTSEC-2026-0004/);
  assert.equal(github.paginate.mock.calls[0].arguments[1].state, 'open');
});
