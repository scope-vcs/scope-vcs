import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';

import { slug } from './agent-fixtures.mjs';
import { percentile, round } from './metrics.mjs';
import { capacityRejectionBreakdown, failureBreakdown } from './railway-load.mjs';

export function stageRow({ mode, concurrency, samples, wallSeconds, startedAt, completedAt }) {
  const succeeded = samples.filter(({ ok }) => ok);
  const latencies = succeeded.map(({ durationMs }) => durationMs);
  return {
    mode,
    concurrency,
    startedAt,
    completedAt,
    operations: samples.length,
    ok: succeeded.length,
    wallSeconds: round(wallSeconds),
    perSec: round(succeeded.length / wallSeconds),
    errorRate: round((samples.length - succeeded.length) / Math.max(1, samples.length)),
    p50: percentile(latencies, 0.5),
    p95: percentile(latencies, 0.95),
    p99: percentile(latencies, 0.99),
    errors: failureBreakdown(samples),
    capacityRejections: capacityRejectionBreakdown(samples),
    sampleError: samples.find(({ ok }) => !ok)?.error ?? null,
  };
}

export function reportPath(outputRoot, label) {
  return join(outputRoot, `${slug(label, 120)}.json`);
}

export async function persistReport(report, path) {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, `${JSON.stringify(report, null, 2)}\n`);
}
