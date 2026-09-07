import { execFileSync, spawnSync } from 'node:child_process';
import { cpSync, mkdirSync, readdirSync, readFileSync, writeFileSync, openSync, closeSync } from 'node:fs';
import { resolve } from 'node:path';
import { sourceSnapshot, saveMetadata, restoreMetadata } from './cache-metadata.mjs';

let root = process.cwd();
const target = resolve(process.env.CARGO_TARGET_DIR);
const output = '/tmp/cache-db-experiment';
mkdirSync(output, { recursive: true });
mkdirSync(target, { recursive: true });
const cacheMode = process.env.SCOPE_CACHE_EXPERIMENT_MODE ?? 'baseline';
const started = performance.now();
if (cacheMode !== 'baseline' && root !== '/workspace') {
  mkdirSync('/workspace', { recursive: true });
  if (readdirSync('/workspace').length) throw new Error('Experiment fixed workspace must start empty');
  cpSync(root, '/workspace', { recursive: true, preserveTimestamps: true });
  process.chdir('/workspace');
  root = process.cwd();
}
const workspaceSeconds = (performance.now() - started) / 1000;
const restoreStarted = performance.now();
const restore = restoreMetadata(root, target, cacheMode);
const restoreSeconds = (performance.now() - restoreStarted) / 1000;
const sources = sourceSnapshot(root);
const env = { ...process.env, SCOPE_DB_EXPERIMENT_METRICS: `${output}/db.jsonl` };
const stages = [];
let exitCode = 0;
console.log('CACHE_DB_RESTORE', JSON.stringify({ cacheMode, root, workspaceSeconds, restoreSeconds, ...restore }));
function run(name, args) {
  const log = `${output}/${name}.log`;
  const fd = openSync(log, 'w');
  const begin = performance.now();
  const child = spawnSync(args[0], args.slice(1), { env, stdio: ['ignore', fd, fd] });
  closeSync(fd);
  const seconds = (performance.now() - begin) / 1000;
  const text = readFileSync(log, 'utf8');
  const summaries = text.split('\n').filter((s) => /Finished |test result:|Dirty |stale:|error:/.test(s));
  const artifacts = { fresh: 0, rebuilt: 0 };
  for (const line of text.split('\n')) {
    if (!line.startsWith('{')) continue;
    let record;
    try { record = JSON.parse(line); } catch { continue; }
    if (record.reason === 'compiler-artifact') artifacts[record.fresh ? 'fresh' : 'rebuilt']++;
  }
  const fingerprintReasons = text.split('\n').filter((s) => /fingerprint.*(dirty|stale|error)|stale:/.test(s));
  const stage = { name, seconds, exitCode: child.status, summaries, artifacts, fingerprintReasonCount: fingerprintReasons.length, fingerprintReasons: fingerprintReasons.slice(0, 25) };
  stages.push(stage);
  console.log('CACHE_DB_STAGE', JSON.stringify(stage));
  if (child.status !== 0) { console.log(text.slice(-16000)); exitCode = child.status ?? 1; }
}
run('fmt', ['cargo', 'fmt', '--all', '--check']);
env.CARGO_LOG = 'cargo::core::compiler::fingerprint=info';
if (!exitCode) run('tests', ['cargo', 'test', '--workspace', '--features', 'api/test-support', '--locked', '--message-format=json']);
if (!exitCode) run('local-dev', ['cargo', 'test', '-p', 'api', '--features', 'local-dev', '--locked', '--message-format=json', 'dev::']);
if (!exitCode) run('clippy', ['cargo', 'clippy', '--workspace', '--features', 'api/test-support', '--all-targets', '--locked', '--message-format=json', '--', '-D', 'warnings']);
const saveStarted = performance.now();
const saved = exitCode ? null : saveMetadata(root, target, sources);
const metadataSaveSeconds = (performance.now() - saveStarted) / 1000;
let db = [];
try { db = readFileSync(env.SCOPE_DB_EXPERIMENT_METRICS, 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse); } catch (error) { if (error.code !== 'ENOENT') throw error; }
const result = {
  source: execFileSync('git', ['rev-parse', 'HEAD']).toString().trim(),
  cacheMode, dbMode: env.SCOPE_DB_EXPERIMENT_MODE, workspaceSeconds, restoreSeconds, metadataSaveSeconds,
  elapsedSeconds: (performance.now() - started) / 1000, exitCode, stages, saved, db,
};
writeFileSync(`${output}/result.json`, JSON.stringify(result, null, 2));
// Full per-store metrics stay in the result artifact, with totals in the run log.
const totals = {};
for (const row of db) {
  const phase = totals[row.phase] ??= { count: 0, sum_ms: 0, values: [] };
  phase.count++;
  phase.sum_ms += row.elapsed_ms;
  phase.values.push(row.elapsed_ms);
}
for (const phase of Object.values(totals)) {
  phase.values.sort((a, b) => a - b);
  phase.mean_ms = phase.sum_ms / phase.count;
  phase.p95_ms = phase.values[Math.ceil(phase.count * 0.95) - 1];
  delete phase.values;
}
console.log('CACHE_DB_RESULT', JSON.stringify({ ...result, db: { records: db.length, totals } }));
process.exitCode = exitCode;
