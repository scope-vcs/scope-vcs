import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';

import { slug } from './agent-fixtures.mjs';
import { percentile, round } from './metrics.mjs';
import { capacityRejectionBreakdown, failureBreakdown } from './railway-load.mjs';

export function latencySummary(samples) {
  const succeeded = samples.filter(({ ok }) => ok);
  const latencies = succeeded.map(({ durationMs }) => durationMs);
  return {
    operations: samples.length,
    ok: succeeded.length,
    errorRate: round((samples.length - succeeded.length) / Math.max(1, samples.length)),
    p50: percentile(latencies, 0.5),
    p95: percentile(latencies, 0.95),
    p99: percentile(latencies, 0.99),
    errors: failureBreakdown(samples),
    sampleError: samples.find(({ ok }) => !ok)?.error ?? null,
  };
}

export function stageRow({ mode, concurrency, samples, wallSeconds, startedAt, completedAt }) {
  const summary = latencySummary(samples);
  return {
    mode,
    concurrency,
    startedAt,
    completedAt,
    ...summary,
    wallSeconds: round(wallSeconds),
    perSec: round(summary.ok / wallSeconds),
    capacityRejections: capacityRejectionBreakdown(samples),
  };
}

export function reportPath(outputRoot, label) {
  return join(outputRoot, `${slug(label, 120)}.json`);
}

export async function persistReport(report, path) {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, `${JSON.stringify(report, null, 2)}\n`);
}
