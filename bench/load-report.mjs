import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';

export async function persistReport(report, output) {
  const json = join(output, 'results.json');
  const markdownPath = join(output, 'summary.md');
  await writeFile(json, `${JSON.stringify(report, null, 2)}\n`);
  await writeFile(markdownPath, markdown(report));
  return { json, markdown: markdownPath };
}

export function markdown(report) {
  const rows = report.workloads.map((workload) => {
    const stage = workload.healthyStage;
    return `| ${workload.name} | ${workload.status} | ${stage?.throughputPerSecond ?? '—'} | ${stage?.normalized.logicalMiBPerSecond ?? '—'} | ${stage?.stats.p95Ms ?? '—'} | ${stage?.stats.ttfbP95Ms ?? '—'} | ${stage?.stats.p99Ms ?? '—'} | ${stage?.normalized.observedMiBPerSecond ?? '—'} |`;
  }).join('\n');
  const unhealthyRows = report.workloads.flatMap((workload) => {
    const stage = workload.stages.find((entry) => !entry.gate.healthy);
    return stage ? [`| ${workload.name} | ${stage.targetRate ?? stage.concurrency} | ${stage.gate.reasons.join('; ')} |`] : [];
  }).join('\n') || '| none | n/a | none |';
  const permits = report.config.apiPermitLimits;
  const rejectionRows = report.workloads.flatMap((workload) => workload.stages.flatMap((stage) =>
    Object.entries(stage.capacityRejections || {}).map(([operation, count]) =>
      `| ${workload.name} | ${stage.concurrency ?? stage.targetRate} | ${operation} | ${count} |`,
    ))).join('\n') || '| none | n/a | none | 0 |';
  return `# Scope Railway Git storage load test

Generated: ${report.generatedAt}

Targets: ${report.apiUrls.join(', ')}

Topology: ${report.config.topologyLabel} (${report.config.routingMode}), repeat ${report.config.repeatIndex}

Repository mode: ${report.config.repositoryMode}

Read replica count: ${report.config.readReplicaCount}

Node scale label: ${report.config.nodeScaleLabel}

Protocol label: ${report.config.protocolLabel}

API permit labels per process: receive-pack ${permits.receivePack}, upload-pack ${permits.uploadPack}, Git materialization ${permits.gitMaterialization}, object store ${permits.objectStore}.

| Workload | Status | Operations/s | Logical MiB/s | Completion p95 ms | TTFB p95 ms | Completion p99 ms | Observed MiB/s |
|---|---|---:|---:|---:|---:|---:|---:|
${rows}

## First unhealthy stages

| Workload | Concurrency or rate | Reasons |
|---|---:|---|
${unhealthyRows}

## Capacity rejections

| Workload | Concurrency or rate | Operation | Count |
|---|---:|---|---:|
${rejectionRows}

Logical MiB/s uses fixture payload sizes for writes and clones, and response or received-object bytes for reads. Observed MiB/s uses response bytes or local Git object deltas. Neither is a wire-level counter. TTFB for JSON reads is time to response headers. Quiet Git commands commonly emit no output, so their completion time is reported as TTFB. Compare topology repeats only when repository fixture sizes, stage controls, and Railway deployment shape are identical.
`;
}
