import assert from 'node:assert/strict';
import test from 'node:test';
import { mutateRailway } from './railway-client.mjs';
import { RailwayRequestError } from './railway-retry.mjs';

const query = 'mutation ConfigureRegistry($input:ServiceInstanceUpdateInput!){serviceInstanceUpdate(input:$input)}';
const variables = { serviceId: 'service-123', input: { registryCredentials: { username: 'registry-user', password: 'registry"secret\\' } } };

function failedProcess(fields) {
  return () => { throw Object.assign(new Error('Command failed: railway api'), fields); };
}

test('a GraphQL error from a failed CLI names its code and message without echoing sent values', () => {
  const stdout = JSON.stringify({ data: null, errors: [
    { message: 'Deploys have been paused temporarily', extensions: { code: 'BAD_USER_INPUT' } },
    { message: `Variable "$input" got invalid value ${JSON.stringify({ password: variables.input.registryCredentials.password })} for service-123` },
  ] });
  assert.throws(() => mutateRailway(query, variables, { execute: failedProcess({ status: 1, stdout }) }), (error) => {
    assert.ok(error instanceof RailwayRequestError);
    assert.equal(error.detail, 'BAD_USER_INPUT: Deploys have been paused temporarily; UNKNOWN: Variable "$input" got invalid value {"password":"[redacted]"} for [redacted]');
    assert.doesNotMatch(error.message, /registry/);
    assert.match(error.message, /^Railway ConfigureRegistry failed: /);
    return true;
  });
});

test('a message that echoes part of a sent value is withheld', () => {
  const stdout = JSON.stringify({ errors: [{ message: 'Invalid password registry"sec...', extensions: { code: 'BAD_USER_INPUT' } }] });
  assert.throws(() => mutateRailway(query, variables, { execute: () => stdout }),
    { detail: 'BAD_USER_INPUT: message withheld because it echoes request values' });
});

test('a CLI that produces no GraphQL response reports how the process ended', () => {
  for (const [fields, detail] of [
    [{ code: 'ETIMEDOUT', signal: 'SIGKILL' }, 'timed out after 120000 ms'],
    [{ signal: 'SIGTERM' }, 'railway CLI killed by SIGTERM'],
    [{ status: 2, stdout: 'registry"secret' }, 'railway CLI exited with status 2'],
  ]) {
    assert.throws(() => mutateRailway(query, variables, { execute: failedProcess(fields) }), { detail });
  }
  assert.throws(() => mutateRailway(query, variables, { execute: () => 'registry"secret' }), { detail: 'response was not JSON' });
});

test('a successful mutation returns the response and keeps secrets on stdin', () => {
  const response = mutateRailway(query, variables, { env: { RAILWAY_API_TOKEN: 'account' }, execute: (command, args, options) => {
    assert.equal(command, 'railway');
    assert.deepEqual(args, ['api', query, '--variables', '@-']);
    assert.deepEqual(JSON.parse(options.input), variables);
    assert.deepEqual(options.env, { RAILWAY_API_TOKEN: 'account' });
    return JSON.stringify({ data: { serviceInstanceUpdate: true } });
  } });
  assert.deepEqual(response, { data: { serviceInstanceUpdate: true } });
});
