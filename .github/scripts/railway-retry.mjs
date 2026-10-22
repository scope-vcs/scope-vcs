export const RAILWAY_MUTATION_TIMEOUT_MS = 120_000;

export class RailwayRequestError extends Error {
  constructor(operation, detail) {
    super(`Railway ${operation} failed: ${detail}`);
    this.detail = detail;
  }
}

export function retryRailway(operation, {
  pause = (milliseconds) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds),
  report = (message) => console.error(message),
} = {}) {
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    try {
      return operation();
    } catch (error) {
      const detail = error instanceof RailwayRequestError ? ` (${error.detail})` : '';
      if (attempt === 3) throw new Error(`Railway request failed after 3 attempts${detail}.`);
      report(`Railway request failed${detail}; retrying (${attempt}/3).`);
      pause(2_000);
    }
  }
}
