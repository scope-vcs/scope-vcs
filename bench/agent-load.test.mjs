import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import {
  agentBranchName, parseMarks, seedFilePath, slug, writeAgentHistoryStream, writeSeedStream,
} from './agent-fixtures.mjs';
import { configuration, mergeBatchSize, publicConfig, seedFileBytes } from './agent-load-config.mjs';
import { reportPath, stageRow } from './agent-load-report.mjs';
import { execute } from './subprocess.mjs';

const REQUIRED = { SCOPE_BENCH_API_URL: 'https://scope-api-loadtest.up.railway.app/', SCOPE_BENCH_AUTH_TOKEN: 'scope_cli_secret' };

test('defaults describe a push run sized to the largest stage', () => {
  const config = configuration(REQUIRED);
  assert.equal(config.apiUrl, 'https://scope-api-loadtest.up.railway.app');
  assert.equal(config.gitUrl, null);
  assert.equal(config.mode, 'push');
  assert.deepEqual(config.stages, [1, 8, 32, 128]);
  assert.equal(config.agents, 128);
  assert.equal(config.stageSeconds, 30);
  assert.equal(config.commits, 40);
  assert.equal(config.deltaBytes, 4096);
  assert.equal(config.mergesPerStage, 40);
  assert.equal(config.prepConcurrency, 16);
  assert.equal(config.repo, null);
  assert.equal(config.seedFiles, 1);
  assert.equal(config.runLabel, 'unlabeled');
  assert.match(config.outputRoot, /\.tmp\/bench\/agent-load$/);
});

test('overrides are parsed and secrets stay out of the public config', () => {
  const config = configuration({
    ...REQUIRED,
    SCOPE_BENCH_GIT_URL: 'https://scope-git-loadtest.up.railway.app/',
    SCOPE_AGENT_MODE: 'merge',
    SCOPE_AGENT_STAGES: '32, 4,4',
    SCOPE_AGENT_COUNT: '36',
    SCOPE_AGENT_MERGES_PER_STAGE: '4',
    SCOPE_AGENT_REPO: 'owner/name',
    SCOPE_AGENT_SEED_FILES: '90000',
    SCOPE_BENCH_RUN_LABEL: 'tree-90k',
  });
  assert.equal(config.gitUrl, 'https://scope-git-loadtest.up.railway.app');
  assert.equal(config.mode, 'merge');
  assert.deepEqual(config.stages, [4, 32]);
  assert.equal(config.agents, 36);
  assert.deepEqual(config.repo, { owner: 'owner', name: 'name' });
  assert.equal(config.seedFileBytes, 64);
  assert.equal(config.runLabel, 'tree-90k');
  assert.equal(JSON.stringify(publicConfig(config)).includes('scope_cli_secret'), false);
});

test('invalid configuration is rejected before any request is made', () => {
  assert.throws(() => configuration({ SCOPE_BENCH_AUTH_TOKEN: 'token' }), /SCOPE_BENCH_API_URL is required/);
  assert.throws(() => configuration({ SCOPE_BENCH_API_URL: REQUIRED.SCOPE_BENCH_API_URL }), /SCOPE_BENCH_AUTH_TOKEN is required/);
  assert.throws(() => configuration({ ...REQUIRED, SCOPE_AGENT_MODE: 'pull' }), /SCOPE_AGENT_MODE must be one of push, merge/);
  assert.throws(() => configuration({ ...REQUIRED, SCOPE_AGENT_STAGES: '1,0' }), /SCOPE_AGENT_STAGES/);
  assert.throws(() => configuration({ ...REQUIRED, SCOPE_AGENT_STAGES: 'a' }), /SCOPE_AGENT_STAGES/);
  assert.throws(() => configuration({ ...REQUIRED, SCOPE_AGENT_COMMITS: '0' }), /SCOPE_AGENT_COMMITS must be a positive integer/);
  assert.throws(() => configuration({ ...REQUIRED, SCOPE_AGENT_SEED_FILES: '-3' }), /SCOPE_AGENT_SEED_FILES must be a positive integer/);
  assert.throws(() => configuration({ ...REQUIRED, SCOPE_AGENT_STAGE_SECONDS: '0' }), /SCOPE_AGENT_STAGE_SECONDS must be positive/);
  assert.throws(() => configuration({ ...REQUIRED, SCOPE_AGENT_REPO: 'just-a-name' }), /SCOPE_AGENT_REPO must be owner\/name/);
  assert.throws(() => configuration({ ...REQUIRED, SCOPE_BENCH_TARGET_KIND: 'production' }), /SCOPE_BENCH_TARGET_KIND must be loadtest or staging/);
  assert.throws(
    () => configuration({ ...REQUIRED, SCOPE_AGENT_STAGES: '1,16', SCOPE_AGENT_COUNT: '8' }),
    /SCOPE_AGENT_COUNT must be at least the largest/,
  );
  assert.throws(
    () => configuration({ ...REQUIRED, SCOPE_AGENT_MODE: 'merge', SCOPE_AGENT_STAGES: '1,128', SCOPE_AGENT_COUNT: '160' }),
    /SCOPE_AGENT_COUNT must be at least 168/,
  );
  assert.doesNotThrow(() => configuration({ ...REQUIRED, SCOPE_AGENT_MODE: 'merge', SCOPE_AGENT_STAGES: '1,128', SCOPE_AGENT_COUNT: '168' }));
  assert.equal(mergeBatchSize(40, 128), 128);
  assert.equal(mergeBatchSize(40, 4), 40);
});

test('agent branch names are unique, bounded, and ref safe', () => {
  assert.match(agentBranchName('Run_1', 7), /^agent-run-1-[0-9a-f]{8}-7$/);
  assert.notEqual(agentBranchName('run_1', 0), agentBranchName('run/1', 0));
  const longLabel = 'a-very-long-label-'.repeat(8);
  const names = Array.from({ length: 200 }, (_, index) => agentBranchName(longLabel, index));
  assert.equal(new Set(names).size, 200);
  assert.ok(names.every((name) => name.length <= 60 && /^[a-z0-9-]+$/.test(name)));
  assert.ok(names[199].endsWith('-199'));
});

test('report file names are derived from a safe label', () => {
  assert.equal(reportPath('/out', 'Hill/Run 2'), '/out/hill-run-2.json');
  assert.equal(slug('A.B'), 'a-b');
});

test('seed layout keeps every directory at or under 1000 entries', () => {
  assert.equal(seedFileBytes(1), 64 * 1024);
  assert.equal(seedFileBytes(2), 64);
  assert.equal(seedFilePath(0, 1), 'seed/000.bin');

  const count = 90_000;
  const entries = new Map();
  const files = new Set();
  for (let index = 0; index < count; index += 1) {
    const path = seedFilePath(index, count);
    files.add(path);
    const segments = path.split('/');
    for (let depth = 0; depth < segments.length - 1; depth += 1) {
      const directory = segments.slice(0, depth + 1).join('/');
      entries.set(directory, (entries.get(directory) ?? new Set()).add(segments[depth + 1]));
    }
  }
  assert.equal(files.size, count);
  assert.ok(Math.max(...[...entries.values()].map((children) => children.size)) <= 1000);
  assert.equal(seedFilePath(89_999, count), 'seed/089/999.bin');
  assert.equal(seedFilePath(1_000, 1_001), 'seed/001/000.bin');
  assert.equal(seedFilePath(5, 1_000), 'seed/005.bin');
  assert.equal(seedFilePath(1_000_000, 1_000_001), 'seed/001/000/000.bin');
});

test('stage rows summarize successful latencies and classify errors', () => {
  const samples = [
    { ok: true, durationMs: 100 },
    { ok: true, durationMs: 300 },
    { ok: false, durationMs: 50, error: 'POST /merge: HTTP 409 conflict' },
    { ok: false, durationMs: 60, error: 'fatal: unable to access: The requested URL returned error: 503' },
  ];
  const row = stageRow({ mode: 'merge', concurrency: 8, samples, wallSeconds: 2, startedAt: 'a', completedAt: 'b' });
  assert.equal(row.operations, 4);
  assert.equal(row.ok, 2);
  assert.equal(row.perSec, 1);
  assert.equal(row.errorRate, 0.5);
  assert.equal(row.p50, 100);
  assert.equal(row.p95, 300);
  assert.deepEqual(row.errors, { 'http-409': 1, 'http-503': 1 });
  assert.match(row.sampleError, /HTTP 409/);
});

test('seed and agent streams import into a repository that borrows base objects', async (context) => {
  const root = await mkdtemp(join(tmpdir(), 'scope-agent-fixture-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  const git = async (args, env = {}, stdinPath) => {
    const result = await execute('git', args, { env, stdinPath, captureStdout: true });
    assert.equal(result.code, 0, result.stderr);
    return result.stdout.trim();
  };

  const base = join(root, 'base');
  await git(['init', '-q', '--bare', base]);
  await writeSeedStream(join(root, 'seed.stream'), 2_500, 64);
  await git(['fast-import', '--quiet'], { GIT_DIR: base }, join(root, 'seed.stream'));
  const mainOid = await git(['rev-parse', 'refs/heads/main'], { GIT_DIR: base });
  const seedFiles = (await git(['ls-tree', '-r', '--name-only', mainOid], { GIT_DIR: base })).split('\n');
  assert.equal(seedFiles.length, 2_500);
  const topLevel = (await git(['ls-tree', mainOid, 'seed/'], { GIT_DIR: base })).split('\n');
  assert.equal(topLevel.length, 3);

  const agent = join(root, 'agent');
  await git(['init', '-q', '--bare', agent]);
  await writeFile(join(agent, 'objects', 'info', 'alternates'), `${join(base, 'objects')}\n`);
  await writeAgentHistoryStream(join(root, 'agent.stream'), { name: 'agent-x-0', baseOid: mainOid, commits: 3, deltaBytes: 100 });
  const marks = join(root, 'agent.marks');
  await git(['fast-import', '--quiet', `--export-marks=${marks}`], { GIT_DIR: agent }, join(root, 'agent.stream'));
  const commits = parseMarks(await readFile(marks, 'utf8'), 3);
  assert.equal(new Set(commits).size, 3);
  assert.equal(await git(['rev-parse', `${commits[0]}^`], { GIT_DIR: agent }), mainOid);
  assert.equal(await git(['rev-parse', `${commits[2]}^`], { GIT_DIR: agent }), commits[1]);
  const added = await git(['diff-tree', '-r', '--name-only', '--no-commit-id', commits[1]], { GIT_DIR: agent });
  assert.equal(added, 'agents/agent-x-0/f-1.bin');
  assert.equal(await git(['cat-file', '-s', `${commits[2]}:agents/agent-x-0/f-2.bin`], { GIT_DIR: agent }), '100');
  assert.throws(() => parseMarks(':1 abc\n', 2), /did not export mark 2/);
});
