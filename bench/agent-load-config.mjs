import { resolve } from 'node:path';

import { enumValue, nonEmpty, positiveInteger, positiveNumber, required } from './env.mjs';
import { parseStages } from './load-config.mjs';
import { validateTargetKind } from './target-safety.mjs';

const MODES = new Set(['push', 'merge']);
const SINGLE_SEED_FILE_BYTES = 64 * 1024;
const TREE_SEED_FILE_BYTES = 64;

export function seedFileBytes(seedFiles) {
  return seedFiles === 1 ? SINGLE_SEED_FILE_BYTES : TREE_SEED_FILE_BYTES;
}

export function configuration(env = process.env) {
  const targetKind = nonEmpty('SCOPE_BENCH_TARGET_KIND', 'loadtest', env);
  validateTargetKind(targetKind);
  const mode = enumValue('SCOPE_AGENT_MODE', 'push', MODES, env);
  const stages = stageList(env.SCOPE_AGENT_STAGES || '1,8,32,128');
  const agents = positiveInteger('SCOPE_AGENT_COUNT', Math.max(...stages), env);
  if (mode === 'push' && agents < Math.max(...stages)) {
    throw new Error('SCOPE_AGENT_COUNT must be at least the largest SCOPE_AGENT_STAGES value in push mode');
  }
  const seedFiles = positiveInteger('SCOPE_AGENT_SEED_FILES', 1, env);
  return {
    apiUrl: required('SCOPE_BENCH_API_URL', env).replace(/\/$/, ''),
    gitUrl: env.SCOPE_BENCH_GIT_URL?.trim().replace(/\/$/, '') || null,
    token: required('SCOPE_BENCH_AUTH_TOKEN', env),
    targetKind,
    mode,
    stages,
    stageSeconds: positiveNumber('SCOPE_AGENT_STAGE_SECONDS', 30, env),
    agents,
    commits: positiveInteger('SCOPE_AGENT_COMMITS', 40, env),
    deltaBytes: positiveInteger('SCOPE_AGENT_DELTA_BYTES', 4096, env),
    mergesPerStage: positiveInteger('SCOPE_AGENT_MERGES_PER_STAGE', 40, env),
    prepConcurrency: positiveInteger('SCOPE_AGENT_PREP_CONCURRENCY', 16, env),
    repo: existingRepository(env.SCOPE_AGENT_REPO),
    seedFiles,
    seedFileBytes: seedFileBytes(seedFiles),
    runLabel: nonEmpty('SCOPE_BENCH_RUN_LABEL', 'unlabeled', env),
    outputRoot: resolve(env.SCOPE_BENCH_OUTPUT_DIR || '.tmp/bench/agent-load'),
  };
}

export function publicConfig(config) {
  const { token: _token, ...safe } = config;
  return safe;
}

function stageList(value) {
  try { return parseStages(value); }
  catch (error) { throw new Error(`SCOPE_AGENT_STAGES ${error.message}`); }
}

function existingRepository(value) {
  const text = value?.trim();
  if (!text) return null;
  const match = /^([^/\s]+)\/([^/\s]+)$/.exec(text);
  if (!match) throw new Error('SCOPE_AGENT_REPO must be owner/name');
  return { owner: match[1], name: match[2] };
}
