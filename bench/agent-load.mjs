#!/usr/bin/env node
import { randomBytes } from 'node:crypto';
import { mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { performance } from 'node:perf_hooks';
import { pathToFileURL } from 'node:url';

import { agentBranchName, parseMarks, slug, writeAgentHistoryStream, writeSeedStream } from './agent-fixtures.mjs';
import { configuration, mergeBatchSize, publicConfig } from './agent-load-config.mjs';
import { persistReport, reportPath, stageRow } from './agent-load-report.mjs';
import { FixtureCleanup } from './fixture-cleanup.mjs';
import { message } from './host.mjs';
import { execute, killCommandTree } from './subprocess.mjs';
import { assertSafeTarget } from './target-safety.mjs';

const COMMAND_TIMEOUT_MS = 300_000;
const STAGE_PAUSE_MS = 5000;
const activeCommands = new Set();

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) await main();

async function main() {
  const config = configuration();
  assertSafeTarget(config.apiUrl, config.targetKind);
  if (config.gitUrl) assertSafeTarget(config.gitUrl, config.targetKind);
  await ready(config);
  const context = { config, interrupted: false };
  const interrupt = () => {
    context.interrupted = true;
    for (const child of activeCommands) killCommandTree(child);
  };
  process.once('SIGINT', interrupt);
  process.once('SIGTERM', interrupt);
  const cleanup = new FixtureCleanup((fixture) => api(config, repoPath(fixture), { method: 'DELETE' }));
  const report = {
    version: 1, generatedAt: new Date().toISOString(), config: publicConfig(config),
    repository: null, seed: null, preparation: null, stages: [], cleanup: null,
  };
  const path = reportPath(config.outputRoot, config.runLabel);
  try {
    const root = cleanup.directory(await mkdtemp(join(tmpdir(), 'scope-agent-load-')), 'fixture');
    const repository = await openRepository(context, cleanup, root, report);
    const agents = await prepareAgents(context, repository, root, report);
    if (config.mode === 'merge') await runMergeStages(context, repository, agents, report);
    else await runPushStages(context, repository, agents, report);
  } finally {
    report.cleanup = await cleanup.run();
    if (report.cleanup.failed.length || context.interrupted) process.exitCode = 1;
    report.completedAt = new Date().toISOString();
    await persistReport(report, path);
    console.log(`results: ${path}`);
  }
}

async function openRepository(context, cleanup, root, report) {
  const { config } = context;
  const base = join(root, 'base');
  let repository;
  if (config.repo) {
    const { owner, name } = config.repo;
    const metadata = await api(config, repoPath({ owner, repo: name }));
    repository = describeRepository(config, owner, name, new URL(metadata.git_remote_url).pathname);
    const started = performance.now();
    await git(['clone', '-q', '--bare', repository.pushUrl, base], { cwd: root, env: authEnvironment(repository.pushUrl, config.token) });
    report.seed = { existing: true, cloneMs: Math.round(performance.now() - started) };
  } else {
    const created = await api(config, '/v1/repos', {
      method: 'POST',
      body: { name: `loadtest-agents-${slug(config.runLabel, 24)}-${randomBytes(3).toString('hex')}`, file_default_view: 'public' },
    });
    repository = describeRepository(config, created.repo.owner_handle, created.repo.name, new URL(created.init.git_remote_url).pathname);
    cleanup.repository({ owner: repository.owner, repo: repository.name });
    report.seed = await seedMain(context, repository, root, base);
  }
  repository.mainOid = await git(['rev-parse', 'refs/heads/main'], { cwd: base });
  repository.base = base;
  report.repository = { owner: repository.owner, name: repository.name, created: !config.repo, mainOid: repository.mainOid };
  console.log(`repository ${repository.owner}/${repository.name} main ${repository.mainOid.slice(0, 12)}`);
  return repository;
}

function describeRepository(config, owner, name, remotePath) {
  return {
    owner, name,
    view: remotePath.split('/')[2],
    apiPath: repoPath({ owner, repo: name }),
    pushUrl: new URL(remotePath, `${config.gitUrl || config.apiUrl}/`).toString(),
  };
}

async function seedMain(context, repository, root, base) {
  const { config } = context;
  const seedDirectory = join(root, 'seed');
  const streamPath = join(root, 'seed.fast-import');
  const started = performance.now();
  await git(['init', '-q', '--bare', seedDirectory]);
  await writeSeedStream(streamPath, config.seedFiles, config.seedFileBytes);
  await git(['fast-import', '--quiet'], { env: { GIT_DIR: seedDirectory }, stdinPath: streamPath });
  const head = await git(['rev-parse', 'refs/heads/main'], { cwd: seedDirectory });
  const generatedMs = Math.round(performance.now() - started);
  const repoConfig = await api(config, `${repository.apiPath}/config`);
  const intent = await api(config, `${repository.apiPath}/push-intents`, {
    method: 'POST',
    body: { head_oid: head, base_config_hash: repoConfig.config_hash, config: repoConfig.config, view: repository.view },
  });
  const pushStarted = performance.now();
  await git(['push', '-q', repository.pushUrl, `${head}:refs/heads/main`], {
    cwd: seedDirectory,
    env: authEnvironment(repository.pushUrl, config.token, [`X-Scope-Push-Intent: ${intent.token}`]),
  });
  const pushMs = Math.round(performance.now() - pushStarted);
  await git(['clone', '-q', '--bare', seedDirectory, base], { cwd: root });
  console.log(`seeded main ${head.slice(0, 12)} with ${config.seedFiles} file(s): generated in ${generatedMs} ms, pushed in ${pushMs} ms`);
  return { existing: false, files: config.seedFiles, fileBytes: config.seedFileBytes, generatedMs, pushMs };
}

async function prepareAgents(context, repository, root, report) {
  const { config } = context;
  const started = performance.now();
  const agents = await mapConcurrent(
    Array.from({ length: config.agents }, (_, index) => index),
    config.prepConcurrency,
    (index) => prepareAgent(context, repository, root, index),
  );
  report.preparation = {
    agents: agents.length, commitsPerAgent: config.commits, deltaBytes: config.deltaBytes,
    ms: Math.round(performance.now() - started),
  };
  console.log(`prepared ${agents.length} agents x ${config.commits} commits in ${report.preparation.ms} ms`);
  return agents;
}

async function prepareAgent(context, repository, root, index) {
  const { config } = context;
  const name = agentBranchName(config.runLabel, index);
  const request = await api(config, `${repository.apiPath}/requests`, {
    method: 'POST', body: { name, title: `Load agent ${index}`, view: repository.view },
  });
  const directory = join(root, `agent-${index}`);
  const streamPath = join(root, `agent-${index}.fast-import`);
  const marksPath = join(root, `agent-${index}.marks`);
  await git(['init', '-q', '--bare', directory]);
  await writeFile(join(directory, 'objects', 'info', 'alternates'), `${join(repository.base, 'objects')}\n`);
  await writeAgentHistoryStream(streamPath, { name, baseOid: repository.mainOid, commits: config.commits, deltaBytes: config.deltaBytes });
  await git(['fast-import', '--quiet', `--export-marks=${marksPath}`], {
    env: { GIT_DIR: directory }, stdinPath: streamPath,
  });
  return {
    index, name, directory, ref: `refs/heads/${name}`,
    requestId: request.request.id,
    commits: parseMarks(await readFile(marksPath, 'utf8'), config.commits),
    next: 0, remoteHead: repository.mainOid,
  };
}

async function pushNext(repository, agent, token) {
  const oid = agent.commits[agent.next];
  const environment = { GIT_DIR: agent.directory, ...authEnvironment(repository.pushUrl, token) };
  const started = performance.now();
  const result = await attempt(
    ['-c', 'push.recurseSubmodules=no', 'push', '-q', `--force-with-lease=${agent.ref}:${agent.remoteHead}`, repository.pushUrl, `${oid}:${agent.ref}`],
    { env: environment },
  );
  const durationMs = performance.now() - started;
  if (result.ok) {
    agent.remoteHead = oid;
    agent.next += 1;
    return { ok: true, durationMs };
  }
  const listing = await attempt(['ls-remote', repository.pushUrl, agent.ref], { env: environment });
  const remote = listing.stdout.split('\t')[0];
  if (remote) {
    agent.remoteHead = remote;
    const position = agent.commits.indexOf(remote);
    if (position >= 0) agent.next = position + 1;
  }
  return { ok: false, durationMs, error: result.error };
}

async function runPushStages(context, repository, agents, report) {
  const { config } = context;
  for (const [position, concurrency] of config.stages.entries()) {
    if (context.interrupted) break;
    const samples = [];
    const startedAt = new Date().toISOString();
    const started = performance.now();
    const deadline = started + config.stageSeconds * 1000;
    const active = [...agents].sort((left, right) => (left.next - right.next) || (left.index - right.index)).slice(0, concurrency);
    await Promise.all(active.map(async (agent) => {
      while (performance.now() < deadline && agent.next < agent.commits.length && !context.interrupted) {
        samples.push(await pushNext(repository, agent, config.token));
      }
    }));
    const row = {
      ...stageRow({ mode: 'push', concurrency, samples, wallSeconds: (performance.now() - started) / 1000, startedAt, completedAt: new Date().toISOString() }),
      exhaustedAgents: active.filter((agent) => agent.next >= agent.commits.length).length,
    };
    record(report, row);
    if (position < config.stages.length - 1) await sleep(STAGE_PAUSE_MS);
  }
}

async function runMergeStages(context, repository, agents, report) {
  const { config } = context;
  const pushStarted = performance.now();
  const pushed = [];
  await mapConcurrent(agents, config.prepConcurrency, async (agent) => {
    const result = await pushNext(repository, agent, config.token);
    if (result.ok) pushed.push(agent);
    else console.log(`push failed for ${agent.name}: ${result.error}`);
  });
  const submitStarted = performance.now();
  const ready = [];
  await mapConcurrent(pushed, config.prepConcurrency, async (agent) => {
    try {
      await api(config, `${repository.apiPath}/requests/${agent.requestId}/submit`, { method: 'POST', body: {} });
      ready.push(agent);
    } catch (error) {
      console.log(`submit failed for ${agent.name}: ${message(error).slice(0, 200)}`);
    }
  });
  report.preparation = {
    ...report.preparation,
    pushedHeads: pushed.length, pushMs: Math.round(submitStarted - pushStarted),
    submitted: ready.length, submitMs: Math.round(performance.now() - submitStarted),
  };
  console.log(`pushed ${pushed.length} request heads in ${report.preparation.pushMs} ms, submitted ${ready.length} in ${report.preparation.submitMs} ms`);
  ready.sort((left, right) => left.index - right.index);
  let cursor = 0;
  for (const concurrency of config.stages) {
    if (context.interrupted) break;
    const batch = ready.slice(cursor, cursor + mergeBatchSize(config.mergesPerStage, concurrency));
    cursor += batch.length;
    const samples = [];
    const startedAt = new Date().toISOString();
    const started = performance.now();
    await mapConcurrent(batch, concurrency, async (agent) => {
      const operationStarted = performance.now();
      try {
        await api(config, `${repository.apiPath}/requests/${agent.requestId}/merge`, {
          method: 'POST', body: { expected_head_oid: agent.remoteHead },
        });
        samples.push({ ok: true, durationMs: performance.now() - operationStarted });
      } catch (error) {
        samples.push({ ok: false, durationMs: performance.now() - operationStarted, error: message(error).slice(0, 300) });
      }
    });
    record(report, stageRow({ mode: 'merge', concurrency, samples, wallSeconds: (performance.now() - started) / 1000, startedAt, completedAt: new Date().toISOString() }));
  }
}

function record(report, row) {
  report.stages.push(row);
  console.log(JSON.stringify(row));
}

async function mapConcurrent(items, concurrency, operation) {
  const queue = [...items];
  const results = [];
  let failure = null;
  await Promise.allSettled(Array.from({ length: Math.min(concurrency, queue.length) }, async () => {
    while (queue.length && !failure) {
      const item = queue.shift();
      try { results.push(await operation(item)); }
      catch (error) { failure ??= error; }
    }
  }));
  if (failure) throw failure;
  return results.sort((left, right) => (left?.index ?? 0) - (right?.index ?? 0));
}

async function attempt(args, options = {}) {
  const execution = await execute('git', args, {
    cwd: options.cwd, env: options.env, stdinPath: options.stdinPath,
    captureStdout: true, timeoutMs: COMMAND_TIMEOUT_MS, activeCommands,
  });
  const ok = execution.code === 0 && !execution.error && !execution.timedOut;
  return { ok, stdout: execution.stdout.trim(), error: ok ? null : failureText(execution) };
}

async function git(args, options) {
  const result = await attempt(args, options);
  if (!result.ok) throw new Error(`git ${args.slice(0, 3).join(' ')}: ${result.error}`);
  return result.stdout;
}

function failureText(execution) {
  const lines = execution.stderr.split('\n').filter((line) => /error|fatal|remote:|!/.test(line));
  const prefix = execution.timedOut ? 'client timeout (SIGKILL): ' : '';
  return `${prefix}${lines.slice(-2).join(' | ') || execution.error || String(execution.signal ?? execution.code)}`.slice(0, 300);
}

function authEnvironment(url, token, extraHeaders = []) {
  const headers = [`Authorization: Bearer ${token}`, ...extraHeaders];
  return Object.fromEntries([
    ['GIT_CONFIG_COUNT', String(headers.length)],
    ...headers.flatMap((header, index) => [
      [`GIT_CONFIG_KEY_${index}`, `http.${url}.extraHeader`],
      [`GIT_CONFIG_VALUE_${index}`, header],
    ]),
  ]);
}

async function api(config, path, options = {}) {
  const headers = { accept: 'application/json', authorization: `Bearer ${config.token}`, 'x-scope-cli-protocol': '1' };
  if (options.body) headers['content-type'] = 'application/json';
  const method = options.method || 'GET';
  const response = await fetch(`${config.apiUrl}${path}`, {
    method, headers, body: options.body ? JSON.stringify(options.body) : undefined,
    signal: AbortSignal.timeout(COMMAND_TIMEOUT_MS),
  });
  const body = await response.text();
  if (!response.ok) throw new Error(`${method} ${path}: HTTP ${response.status} ${body.slice(0, 300)}`);
  return body ? JSON.parse(body) : null;
}

async function ready(config) {
  for (const endpoint of [config.apiUrl, config.gitUrl].filter(Boolean)) {
    const response = await fetch(`${endpoint}/readyz`, { signal: AbortSignal.timeout(30_000) });
    if (!response.ok) throw new Error(`${endpoint} is not ready: HTTP ${response.status}`);
  }
}

function repoPath(fixture) {
  return `/v1/repos/${encodeURIComponent(fixture.owner)}/${encodeURIComponent(fixture.repo)}`;
}

function sleep(milliseconds) { return new Promise((resolveSleep) => setTimeout(resolveSleep, milliseconds)); }
