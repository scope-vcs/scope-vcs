import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import test from 'node:test';

const script = resolve(import.meta.dirname, 'fetch-ci-history.sh');
const git = (cwd, ...args) => execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trim();

test('shallow PR merges and Scope pushes recover distant ancestry without fetching unrelated refs or tags', (t) => {
  const directory = mkdtempSync(resolve(tmpdir(), 'scope-ci-history-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const remote = resolve(directory, 'remote');
  git(directory, 'init', '--bare', remote);
  const commits = [
    'blob\nmark :1\ndata 4\nbase',
    'commit refs/heads/main\nmark :2\ncommitter Fixture <fixture@scope.test> 1000000000 +0000\ndata 4\nbase\nM 100644 :1 base\n',
  ];
  for (let index = 0; index < 128; index += 1) {
    commits.push(`commit refs/heads/main\ncommitter Fixture <fixture@scope.test> ${1000000001 + index} +0000\ndata 4\nmain\n`);
  }
  commits.push('commit refs/heads/scope/requests/req_old\nmark :3\ncommitter Fixture <fixture@scope.test> 1000000200 +0000\ndata 7\nrequest\nfrom :2\nM 100644 :1 web/request.ts\n');
  commits.push('commit refs/heads/pr-merge\ncommitter Fixture <fixture@scope.test> 1000000201 +0000\ndata 5\nmerge\nfrom refs/heads/main\nmerge :3\nM 100644 :1 web/request.ts\n');
  commits.push('commit refs/heads/unrelated\ncommitter Fixture <fixture@scope.test> 1000000202 +0000\ndata 9\nunrelated\nfrom :2\nM 100644 :1 unrelated\n');
  execFileSync('git', ['fast-import', '--quiet'], { cwd: remote, input: `${commits.join('\n')}\ndone\n`, stdio: ['pipe', 'pipe', 'pipe'] });
  git(remote, 'tag', 'unrelated-tag', 'unrelated');
  const head = git(remote, 'rev-parse', 'scope/requests/req_old');
  const base = git(remote, 'rev-parse', 'main');
  const mergeBase = git(remote, 'merge-base', 'main', head);
  const unrelated = git(remote, 'rev-parse', 'unrelated');

  for (const [event, branch] of [['pull_request', 'pr-merge'], ['push', 'scope/requests/req_old']]) {
    const checkout = resolve(directory, event);
    git(directory, 'clone', '--depth=1', '--no-tags', '--branch', branch, pathToFileURL(remote).href, checkout);
    assert.equal(git(checkout, 'rev-parse', '--is-shallow-repository'), 'true');
    execFileSync('bash', [script], {
      cwd: checkout,
      env: { ...process.env, EVENT_NAME: event, PULL_REQUEST_BASE_SHA: base, HEAD_SHA: head },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    const comparison = event === 'pull_request' ? base : git(checkout, 'merge-base', 'origin/main', 'HEAD');
    assert.equal(git(checkout, 'merge-base', comparison, head), mergeBase);
    assert.equal(git(checkout, 'diff', '--name-only', `${comparison}...${head}`), 'web/request.ts');
    assert.equal(git(checkout, 'rev-parse', '--is-shallow-repository'), 'false');
    assert.equal(git(checkout, 'tag', '--list'), '');
    assert.notEqual(spawnSync('git', ['cat-file', '-e', unrelated], { cwd: checkout }).status, 0);
  }
});

test('history fetch retries a timeout once within the plan budget and stops after a second failure', (t) => {
  const directory = mkdtempSync(resolve(tmpdir(), 'scope-ci-fetch-retry-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const bin = resolve(directory, 'bin');
  mkdirSync(bin);
  writeFileSync(resolve(bin, 'git'), '#!/bin/sh\ncase "$2" in HEAD) printf "%040d\\n" 1 ;; --is-shallow-repository) echo true ;; *) exit 1 ;; esac\n', { mode: 0o755 });
  writeFileSync(resolve(bin, 'timeout'), '#!/bin/sh\nprintf "%s\\n" "$*" >> "$FETCH_CALLS"\nif [ "$FETCH_RECOVER" = true ] && [ -f "$FETCH_FAILED" ]; then exit 0; fi\ntouch "$FETCH_FAILED"\nexit 124\n', { mode: 0o755 });
  for (const recover of [true, false]) {
    const calls = resolve(directory, `calls-${recover}`);
    const result = spawnSync('bash', [script], {
      env: {
        ...process.env, PATH: `${bin}:${process.env.PATH}`, EVENT_NAME: 'push', HEAD_SHA: 'a'.repeat(40),
        FETCH_CALLS: calls, FETCH_RECOVER: String(recover), FETCH_FAILED: resolve(directory, `failed-${recover}`),
      },
      encoding: 'utf8',
    });
    assert.equal(result.status === 0, recover, result.stderr);
    const attempts = readFileSync(calls, 'utf8').trim().split('\n');
    assert.equal(attempts.length, 2);
    for (const attempt of attempts) {
      assert.match(attempt, /^--kill-after=5s 45s git fetch --no-tags --no-recurse-submodules --no-auto-maintenance --unshallow origin [a-f0-9]{40} [a-f0-9]{40} \+refs\/heads\/main:refs\/remotes\/origin\/main$/);
    }
  }
});
