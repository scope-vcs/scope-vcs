import { execFileSync } from 'node:child_process';
import { readRailway } from './railway-read.mjs';
import { RAILWAY_MUTATION_TIMEOUT_MS, RailwayRequestError } from './railway-retry.mjs';

const MESSAGE_LIMIT = 300;
const ECHO_WINDOW = 8;

function stringLeaves(value) {
  if (typeof value === 'string') return [value];
  if (value && typeof value === 'object') return Object.values(value).flatMap(stringLeaves);
  return [];
}

function echoesSentValue(text, sent) {
  return sent.some(value => {
    for (let start = 0; start + ECHO_WINDOW <= value.length; start += 1) {
      if (text.includes(value.slice(start, start + ECHO_WINDOW))) return true;
    }
    return false;
  });
}

function redact(message, variables) {
  const sent = stringLeaves(variables).filter(Boolean)
    .flatMap(value => [value, JSON.stringify(value).slice(1, -1)])
    .sort((left, right) => right.length - left.length);
  const redacted = sent.reduce((text, value) => text.split(value).join('[redacted]'), String(message ?? ''));
  if (echoesSentValue(redacted, sent)) return 'message withheld because it echoes request values';
  return redacted.replace(/\s+/g, ' ').trim().slice(0, MESSAGE_LIMIT) || 'no message';
}

export function describeRailwayErrors(errors, variables) {
  return errors.map(error => `${error?.extensions?.code ?? 'UNKNOWN'}: ${redact(error?.message, variables)}`).join('; ');
}

function parseResponse(output) {
  try {
    const response = JSON.parse(output);
    return response && typeof response === 'object' ? response : undefined;
  } catch {
    return undefined;
  }
}

function processFailure(error) {
  if (error?.code === 'ETIMEDOUT') return `timed out after ${RAILWAY_MUTATION_TIMEOUT_MS} ms`;
  if (error?.signal) return `railway CLI killed by ${error.signal}`;
  if (Number.isInteger(error?.status)) return `railway CLI exited with status ${error.status}`;
  return 'railway CLI did not run';
}

export function mutateRailway(query, variables, { execute = execFileSync, env } = {}) {
  const operation = query.match(/^mutation\s+(\w+)/)?.[1] ?? 'mutation';
  let output;
  let failure;
  try {
    output = execute('railway', ['api', query, '--variables', '@-'], {
      input: JSON.stringify(variables), encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'],
      timeout: RAILWAY_MUTATION_TIMEOUT_MS, killSignal: 'SIGKILL', ...(env && { env }),
    });
  } catch (error) {
    output = error?.stdout;
    failure = processFailure(error);
  }
  const response = parseResponse(output);
  if (response?.errors?.length) throw new RailwayRequestError(operation, describeRailwayErrors(response.errors, variables));
  if (failure) throw new RailwayRequestError(operation, failure);
  if (!response) throw new RailwayRequestError(operation, 'response was not JSON');
  return response;
}

export function railwayClient() {
  return {
    query(query, variables) {
      return readRailway(['api', query, '--variables', '@-'], { input: JSON.stringify(variables) }).data;
    },
    mutate(query, variables) {
      return mutateRailway(query, variables).data;
    },
    pause(milliseconds) {
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
    },
    now: () => Date.now(),
  };
}
