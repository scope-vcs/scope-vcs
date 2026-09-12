import { resolve } from 'node:path';

import { ROUTING_MODES, createEndpointRouter, parseApiUrls } from './endpoint-routing.mjs';
import {
  boundedNumber, enumValue, nonEmpty, nonNegativeNumber, positiveInteger, positiveNumber, required,
} from './env.mjs';
import { validateRepositoryMode } from './repository-mode.mjs';
import { validateTargetKind } from './target-safety.mjs';
import { parseChangedFileCounts } from './write-shape.mjs';

const DEFAULT_STAGES = [1, 2, 4, 8];
const DEFAULT_WORKLOADS = [
  'warm-fetch', 'incremental-fetch', 'full-clone', 'code-read', 'repo-read',
  'projection-read', 'tree-read', 'blob-read', 'history-read', 'cold-churn', 'mixed', 'consistency',
];
const SUPPORTED_WORKLOADS = new Set([...DEFAULT_WORKLOADS, 'push-persistence']);

export function configuration() {
  const api = required('SCOPE_BENCH_API_URL').replace(/\/$/, '');
  const apiUrls = parseApiUrls(api, process.env.SCOPE_BENCH_API_URLS);
  const routingMode = nonEmpty('SCOPE_LOAD_ROUTING_MODE', 'single');
  if (!ROUTING_MODES.has(routingMode)) throw new Error(`SCOPE_LOAD_ROUTING_MODE must be one of ${[...ROUTING_MODES].join(', ')}`);
  const routingSeed = positiveInteger('SCOPE_LOAD_ROUTING_SEED', 1);
  const token = required('SCOPE_BENCH_AUTH_TOKEN');
  const stages = parseStages(process.env.SCOPE_LOAD_STAGES || DEFAULT_STAGES.join(','));
  const writeDeltaBytes = parseByteSizes(process.env.SCOPE_LOAD_WRITE_DELTA_BYTES || String(64 * 1024));
  const landingFileBytes = parseByteSizes(process.env.SCOPE_LOAD_LANDING_FILE_BYTES || '0');
  const changedFileCounts = parseChangedFileCounts(process.env.SCOPE_LOAD_CHANGED_FILE_COUNTS || '0');
  const workloads = workloadList('SCOPE_LOAD_WORKLOADS', DEFAULT_WORKLOADS);
  const repositoryMode = nonEmpty('SCOPE_LOAD_REPOSITORY_MODE', 'spread');
  const targetKind = nonEmpty('SCOPE_BENCH_TARGET_KIND', 'loadtest');
  validateTargetKind(targetKind);
  const historyDepths = parseStages(process.env.SCOPE_LOAD_HISTORY_DEPTHS || '1,16,64');
  validateRepositoryMode(repositoryMode, workloads, historyDepths, process.env.SCOPE_LOAD_READ_REPLICA_COUNT);
  return {
    apiUrls, gitUrl: process.env.SCOPE_BENCH_GIT_URL?.trim().replace(/\/$/, '') || null,
    token, stages, routingMode, routingSeed, targetKind,
    endpointRouter: createEndpointRouter(apiUrls, routingMode, routingSeed),
    rates: process.env.SCOPE_LOAD_RATES ? parseRates(process.env.SCOPE_LOAD_RATES) : null,
    workloads, repositoryMode,
    stageSeconds: positiveNumber('SCOPE_LOAD_STAGE_SECONDS', 120),
    warmupSeconds: nonNegativeNumber('SCOPE_LOAD_WARMUP_SECONDS', 0),
    warmupConcurrency: positiveInteger('SCOPE_LOAD_WARMUP_CONCURRENCY', 4),
    confirmSeconds: nonNegativeNumber('SCOPE_LOAD_CONFIRM_SECONDS', 300),
    timeoutMs: positiveNumber('SCOPE_LOAD_TIMEOUT_MS', 90_000),
    cleanupTimeoutMs: positiveNumber('SCOPE_LOAD_CLEANUP_TIMEOUT_MS', 10_000),
    maxInFlight: positiveInteger('SCOPE_LOAD_MAX_IN_FLIGHT', 128),
    churnRepos: positiveInteger('SCOPE_LOAD_CHURN_REPOS', 16),
    mixedRepos: Math.max(writeDeltaBytes.length, landingFileBytes.length, changedFileCounts.length, positiveInteger('SCOPE_LOAD_MIXED_REPOS', Math.max(8, ...stages))),
    readBytes: positiveInteger('SCOPE_LOAD_READ_BYTES', 384 * 1024),
    writeDeltaBytes,
    landingFileBytes,
    changedFileCounts,
    changedFileBytes: positiveInteger('SCOPE_LOAD_CHANGED_FILE_BYTES', 4096),
    pushPath: enumValue('SCOPE_LOAD_PUSH_PATH', 'focused', new Set(['focused', 'aggregate'])),
    pushBaselineP95Ms: process.env.SCOPE_LOAD_PUSH_BASELINE_P95_MS
      ? positiveNumber('SCOPE_LOAD_PUSH_BASELINE_P95_MS', 1)
      : null,
    historyDepths,
    mixedWritePercent: boundedNumber('SCOPE_LOAD_MIXED_WRITE_PERCENT', 20, 0, 100),
    apiPermitLimits: {
      receivePack: positiveInteger('SCOPE_BENCH_RECEIVE_PACK_CONCURRENCY', 4),
      uploadPack: positiveInteger('SCOPE_BENCH_UPLOAD_PACK_CONCURRENCY', 8),
      gitMaterialization: positiveInteger('SCOPE_BENCH_GIT_MATERIALIZATION_CONCURRENCY', 2),
      objectStore: positiveInteger('SCOPE_BENCH_OBJECT_STORE_CONCURRENCY', 16),
    },
    nodeScaleLabel: nonEmpty('SCOPE_LOAD_NODE_SCALE_LABEL', 'unspecified'),
    readReplicaCount: positiveInteger('SCOPE_LOAD_READ_REPLICA_COUNT', 1),
    protocolLabel: nonEmpty('SCOPE_LOAD_PROTOCOL_LABEL', 'current'),
    runLabel: nonEmpty('SCOPE_BENCH_RUN_LABEL', 'unlabeled'),
    topologyLabel: nonEmpty('SCOPE_LOAD_TOPOLOGY_LABEL', routingMode),
    repeatIndex: positiveInteger('SCOPE_LOAD_REPEAT_INDEX', 1),
    consistencyTimeoutMs: positiveNumber('SCOPE_LOAD_CONSISTENCY_TIMEOUT_MS', 30_000),
    consistencyPollMs: positiveNumber('SCOPE_LOAD_CONSISTENCY_POLL_MS', 50),
    outputRoot: resolve(process.env.SCOPE_BENCH_OUTPUT_DIR || '.tmp/bench/railway-load'),
  };
}

export function publicConfig(config) {
  const { token: _token, endpointRouter: _endpointRouter, ...safe } = config;
  return safe;
}

export function parseStages(value) {
  const stages = [...new Set(value.split(',').map((entry) => Number.parseInt(entry.trim(), 10)))];
  if (!stages.length || stages.some((stage) => !Number.isInteger(stage) || stage < 1)) throw new Error('value must be a comma-separated list of positive integers');
  return stages.sort((left, right) => left - right);
}

export function parseRates(value) {
  const rates = [...new Set(value.split(',').map((entry) => Number(entry.trim())))];
  if (!rates.length || rates.some((rate) => !Number.isFinite(rate) || rate <= 0)) throw new Error('value must be a comma-separated list of positive numbers');
  return rates.sort((left, right) => left - right);
}

export function parseByteSizes(value) {
  const entries = value.split(',').map((entry) => entry.trim());
  const sizes = [...new Set(entries.map((entry) => /^\d+$/.test(entry) ? Number(entry) : Number.NaN))];
  if (!sizes.length || sizes.some((size) => !Number.isSafeInteger(size) || size < 0)) throw new Error('SCOPE_LOAD_WRITE_DELTA_BYTES must be a comma-separated list of non-negative byte counts');
  return sizes.sort((left, right) => left - right);
}

function workloadList(name, fallback) {
  const values = (process.env[name] || fallback.join(',')).split(',').map((value) => value.trim()).filter(Boolean);
  const unknown = values.filter((value) => !SUPPORTED_WORKLOADS.has(value));
  if (unknown.length) throw new Error(`${name} has unsupported workloads: ${unknown.join(', ')}`);
  return [...new Set(values)];
}
