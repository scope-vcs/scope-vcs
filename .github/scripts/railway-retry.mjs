// Call only for reads or operations that set the same desired state on every try.
export function retryRailway(operation, {
  pause = (milliseconds) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds),
  report = (message) => console.error(message),
} = {}) {
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    try {
      return operation();
    } catch {
      if (attempt === 3) throw new Error('Railway request failed after 3 attempts.');
      report(`Railway request failed; retrying (${attempt}/3).`);
      pause(2_000);
    }
  }
}
