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

export function parseRailwayResponse(output) {
  try {
    const response = JSON.parse(output);
    return response && typeof response === 'object' ? response : undefined;
  } catch {
    return undefined;
  }
}

export function describeProcessFailure(error, timeoutMilliseconds) {
  if (error?.code === 'ETIMEDOUT') return `timed out after ${timeoutMilliseconds} ms`;
  if (error?.signal) return `railway CLI killed by ${error.signal}`;
  if (Number.isInteger(error?.status)) return `railway CLI exited with status ${error.status}`;
  return 'railway CLI did not run';
}

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
