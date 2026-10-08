import { execFileSync } from 'node:child_process';
import { readRailway } from './railway-read.mjs';
import { RAILWAY_MUTATION_TIMEOUT_MS } from './railway-retry.mjs';

export function railwayClient() {
  return {
    query(query, variables) {
      return readRailway(['api', query, '--variables', '@-'], { input: JSON.stringify(variables) }).data;
    },
    mutate(query, variables) {
      let result;
      try {
        result = JSON.parse(execFileSync('railway', ['api', query, '--variables', '@-'], {
          input: JSON.stringify(variables), encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'],
          timeout: RAILWAY_MUTATION_TIMEOUT_MS, killSignal: 'SIGKILL',
        }));
      } catch {
        throw new Error(`Railway ${query.split('(')[0]} failed.`);
      }
      if (result.errors?.length) throw new Error(`Railway ${query.split('(')[0]} failed.`);
      return result.data;
    },
    pause(milliseconds) {
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
    },
    now: () => Date.now(),
  };
}
