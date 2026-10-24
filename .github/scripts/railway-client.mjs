import { execFileSync } from 'node:child_process';
import { readRailway } from './railway-read.mjs';
import {
  RAILWAY_MUTATION_TIMEOUT_MS, RailwayRequestError, describeProcessFailure, describeRailwayErrors, parseRailwayResponse,
} from './railway-retry.mjs';

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
    failure = describeProcessFailure(error, RAILWAY_MUTATION_TIMEOUT_MS);
  }
  const response = parseRailwayResponse(output);
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
