import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { cpus, totalmem } from 'node:os';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';

const output = resolve(process.env.BENCH_OUTPUT ?? '/tmp/runner-economics');
mkdirSync(output, { recursive: true });
const read = (path) => {
  try { return readFileSync(path, 'utf8').trim(); } catch { return null; }
};
const commandOutput = (command, args) => {
  const result = spawnSync(command, args, { encoding: 'utf8' });
  return result.status === 0 ? result.stdout.trim() : null;
};
const network = () => Object.fromEntries(
  (read('/proc/net/dev') ?? '').split('\n').slice(2).map((line) => {
    const [name, values] = line.trim().split(':');
    const fields = values.trim().split(/\s+/).map(Number);
    return [name.trim(), { receivedBytes: fields[0], sentBytes: fields[8] }];
  }),
);
const snapshot = () => ({
  at: new Date().toISOString(),
  network: network(),
  cpuStat: read('/sys/fs/cgroup/cpu.stat'),
  cpuMax: read('/sys/fs/cgroup/cpu.max'),
  memoryMax: read('/sys/fs/cgroup/memory.max'),
  memoryPeak: read('/sys/fs/cgroup/memory.peak'),
  ioStat: read('/sys/fs/cgroup/io.stat'),
});
const command = process.argv.slice(2);
if (!command.length) command.push('bash', './dev/checks/backend', 'with-api');
const metadata = {
  runner: process.env.BENCH_RUNNER ?? 'unspecified',
  cache: process.env.BENCH_CACHE ?? 'unspecified',
  sample: process.env.BENCH_SAMPLE ?? 'unspecified',
  sourceSha: commandOutput('git', ['rev-parse', 'HEAD']),
  rust: commandOutput('rustc', ['--version']),
  cpus: cpus().map(({ model }) => model),
  memoryBytes: totalmem(),
  command,
};
const before = snapshot();
const start = performance.now();
const child = spawnSync('/usr/bin/time', ['-v', '-o', `${output}/time.txt`, ...command], {
  stdio: 'inherit',
});
const elapsedSeconds = (performance.now() - start) / 1000;
const result = {
  ...metadata, before, after: snapshot(), elapsedSeconds,
  exitCode: child.status, signal: child.signal, error: child.error?.message ?? null,
  resourceUsage: read(`${output}/time.txt`),
};
writeFileSync(`${output}/result.json`, `${JSON.stringify(result, null, 2)}\n`);
console.log(`RUNNER_ECONOMICS_RESULT ${JSON.stringify(result)}`);
process.exitCode = child.status ?? 1;
