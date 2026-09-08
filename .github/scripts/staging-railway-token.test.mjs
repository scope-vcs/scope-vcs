import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

function run(t, scenario, action = 'create') {
  const root = mkdtempSync(join(tmpdir(), 'railway-token-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  writeFileSync(join(root, 'state'), JSON.stringify({ tokens: action === 'delete' ? ['token-id'] : [], calls: [] }));
  writeFileSync(join(root, 'sleep'), '#!/bin/sh\nexit 0\n', { mode: 0o755 });
  writeFileSync(join(root, 'curl'), `#!/usr/bin/env node
const fs = require('node:fs');
const file = process.env.MOCK_STATE;
const state = JSON.parse(fs.readFileSync(file, 'utf8'));
const request = JSON.parse(process.argv[process.argv.indexOf('--data-binary') + 1]);
const operation = request.query.match(/(?:query|mutation) (\\w+)/)[1];
state.calls.push(operation);
let response;
let failure = false;
if (operation === 'ProjectTokens') {
  failure = process.env.MOCK_SCENARIO === 'list-fails' ||
    (process.env.MOCK_SCENARIO === 'list-transient' && state.calls.length === 1);
  response = failure ? { data: { projectTokens: { edges: [] } } } :
    { data: { projectTokens: { edges: state.tokens.map(id => ({ node: { id, name: 'run-name' } })) } } };
  if (process.env.MOCK_SCENARIO === 'missing-list') response = { data: {} };
} else if (operation === 'ProjectTokenCreate') {
  state.tokens.push('token-id');
  response = { data: { projectTokenCreate: 'SECRET_CREATED_TOKEN' } };
  failure = process.env.MOCK_SCENARIO === 'create-ambiguous';
} else if (operation === 'ProjectTokenDelete') {
  state.tokens = [];
  response = { data: { projectTokenDelete: true } };
  failure = process.env.MOCK_SCENARIO === 'delete-ambiguous';
}
fs.writeFileSync(file, JSON.stringify(state));
process.stdout.write(JSON.stringify(response));
if (failure) { process.stderr.write('SECRET provider diagnostics'); process.exitCode = 22; }
`, { mode: 0o755 });
  const result = spawnSync('bash', ['.github/scripts/staging-railway-token.sh', action], {
    encoding: 'utf8', timeout: 10_000,
    env: {
      ...process.env, PATH: `${root}:${process.env.PATH}`,
      MOCK_STATE: join(root, 'state'), MOCK_SCENARIO: scenario,
      SCOPE_RAILWAY_PROJECT_ID: 'project', SCOPE_RAILWAY_STAGING_ENVIRONMENT_ID: 'staging',
      SCOPE_RAILWAY_PROJECT_TOKEN_NAME: 'run-name', RAILWAY_API_TOKEN: 'account-token',
      GITHUB_ENV: join(root, 'github-env'),
    },
  });
  return { result, state: JSON.parse(readFileSync(join(root, 'state'), 'utf8')), root };
}

test('failed token list cannot become an empty list inside the create guard', (t) => {
  const { result, state } = run(t, 'list-fails');
  assert.equal(result.status, 1, result.stderr);
  assert.deepEqual(state.calls, ['ProjectTokens', 'ProjectTokens', 'ProjectTokens']);
  assert.doesNotMatch(result.stdout + result.stderr, /SECRET/);
});

test('missing token list data fails closed before creation', (t) => {
  const { result, state } = run(t, 'missing-list');
  assert.equal(result.status, 1);
  assert.deepEqual(state.calls, ['ProjectTokens']);
});

test('transient read failure retries before creating exactly one token', (t) => {
  const { result, state, root } = run(t, 'list-transient');
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(state.calls, ['ProjectTokens', 'ProjectTokens', 'ProjectTokenCreate', 'ProjectTokens']);
  assert.equal(readFileSync(join(root, 'github-env'), 'utf8'), 'RAILWAY_TOKEN=SECRET_CREATED_TOKEN\n');
});

test('ambiguous successful creation is cleaned up without repeating or leaking creation', (t) => {
  const { result, state } = run(t, 'create-ambiguous');
  assert.equal(result.status, 1, result.stderr);
  assert.deepEqual(state.tokens, []);
  assert.deepEqual(state.calls, ['ProjectTokens', 'ProjectTokenCreate', 'ProjectTokens', 'ProjectTokenDelete', 'ProjectTokens']);
  assert.doesNotMatch(result.stdout + result.stderr, /SECRET/);
});

test('ambiguous successful deletion is confirmed without a duplicate mutation', (t) => {
  const { result, state } = run(t, 'delete-ambiguous', 'delete');
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(state.calls, ['ProjectTokens', 'ProjectTokenDelete', 'ProjectTokens']);
  assert.deepEqual(state.tokens, []);
});

test('failed delete list cannot report that cleanup succeeded', (t) => {
  const { result, state } = run(t, 'list-fails', 'delete');
  assert.equal(result.status, 1, result.stderr);
  assert.deepEqual(state.tokens, ['token-id']);
  assert.equal(state.calls.length, 3);
});
