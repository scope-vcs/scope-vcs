import assert from 'node:assert/strict';
import { mock, test } from 'node:test';
import { advisoryIssueBody, checkMainAdvisories, imageScanCheck, reportMainAdvisories } from './main-advisories.mjs';

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

test('every lockfile is checked after an advisory failure or a process launch failure', () => {
  for (const failure of [{status: 1, stderr: 'advisory failure'}, {status: null, error: new Error('spawn failed')}]) {
    const run = mock.fn(() => failure);
    assert.deepEqual(checkMainAdvisories(run).map(check => [check.target, check.status]),
      ['workspace', 'cli', 'web'].map(target => [target, failure.status]));
    assert.equal(run.mock.callCount(), 3);
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
  const web = advisoryIssueBody({checks: [{target: 'web', status: 1, output: '1 vulnerabilities found\nSeverity: 1 high'}], runUrl, sha});
  assert.match(web, /### web: failed\n[\s\S]*No advisory IDs[\s\S]*Severity: 1 high/);
});

test('the issue lists the image findings that failed the scan, or why the scan has none to list', () => {
  const finding = { VulnerabilityID: 'CVE-2026-0001', PkgName: 'libssl3', Severity: 'HIGH', InstalledVersion: '1', FixedVersion: '2' };
  const report = findings => ({
    SchemaVersion: 2, Metadata: { OS: { Family: 'debian' } },
    Results: [{ Class: 'os-pkgs', Type: 'debian', Target: 'Debian', Vulnerabilities: findings }],
  });
  const body = scan => advisoryIssueBody({checks: [imageScanCheck(scan)], runUrl, sha});
  const found = body(report([finding, { ...finding, VulnerabilityID: 'CVE-2026-0002', Severity: 'LOW' }]));
  assert.match(found, /### media worker image: failed\n\n- CVE-2026-0001: `libssl3 1`, fixed in 2\n/);
  assert.doesNotMatch(found, /CVE-2026-0002|No advisory IDs/);
  assert.match(body(report([])), /no fixable HIGH or CRITICAL finding/);
  assert.match(body(null), /Expected a complete Trivy Debian executable-image report/);
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
