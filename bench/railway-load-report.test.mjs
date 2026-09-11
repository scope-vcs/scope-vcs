import assert from 'node:assert/strict';
import test from 'node:test';
import { runStaircase, markdown } from './railway-load.mjs';

const config = { stages: [1, 2], stageSeconds: 0.025, confirmSeconds: 0, warmupSeconds: 0,
  nodeScaleLabel: 'test', apiPermitLimits: {}, topologyLabel: 'test' };

test('staircase summary uses the selected healthy stage for throughput and every latency/byte column', async () => {
  let failing = false;
  const workload = await runStaircase('repo-read', { config, interrupted: () => false }, async (worker) => {
    if (worker === 1) failing = true;
    await new Promise((resolve) => setTimeout(resolve, 1));
    return { ok: !failing, durationMs: failing ? 90 : 10, ttfbMs: failing ? 80 : 5,
      bytes: failing ? 900 : 100, logicalBytes: failing ? 9000 : 1000 };
  });
  assert.equal(workload.stages.length, 2);
  assert.equal(workload.stages[0].gate.healthy, true);
  assert.equal(workload.stages[1].gate.healthy, false);
  assert.equal(workload.healthyStage, workload.stages[0]);
  const report = { generatedAt: 'now', apiUrls: ['local'], config, workloads: [workload] };
  assert.match(markdown(report), /\| repo-read \| 2 \| error rate 100.00% > 1%/);
  const row = markdown(report).split('\n').find((line) => line.startsWith('| repo-read |'));
  const stage = workload.stages[0];
  assert.equal(row, `| repo-read | measured | ${stage.throughputPerSecond} | ${stage.normalized.logicalMiBPerSecond} | 10 | 5 | 10 | ${stage.normalized.observedMiBPerSecond} |`);
});

test('a wholly unhealthy staircase has no claimed healthy metrics', async () => {
  const workload = await runStaircase('repo-read', { config, interrupted: () => false }, async () => {
    await new Promise((resolve) => setTimeout(resolve, 1));
    return { ok: false, durationMs: 90, bytes: 900 };
  });
  assert.equal(workload.status, 'failed');
  assert.equal(workload.healthyStage, null);
  const row = markdown({ generatedAt: 'now', apiUrls: ['local'], config, workloads: [workload] })
    .split('\n').find((line) => line.startsWith('| repo-read |'));
  assert.equal(row, '| repo-read | failed | — | — | — | — | — | — |');
});
