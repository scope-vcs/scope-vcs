import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

const runner = new URL('./run-bounded-image-builds.sh', import.meta.url).pathname;

function run(components, delayWait = false) {
  const directory = mkdtempSync(join(tmpdir(), 'scope-image-pool-'));
  const builder = join(directory, 'builder.sh');
  const events = join(directory, 'events.log');
  const delayedRunner = join(directory, 'delayed-runner.sh');
  // Pause the parent at the wait boundary while both builder processes exit.
  writeFileSync(delayedRunner, readFileSync(runner, 'utf8').replace(
    'wait_for_one() {', 'wait_for_one() {\n  sleep 0.3'));
  writeFileSync(builder, `#!/usr/bin/env bash
printf 'start %s\\n' "$1" >> "$BUILD_EVENTS"
[[ "$1" != fastfail ]] || exit 4
if [[ "$1" == slowlong ]]; then sleep 0.8; else sleep 0.15; fi
printf 'end %s\\n' "$1" >> "$BUILD_EVENTS"
`);
  const result = spawnSync('bash', [delayWait ? delayedRunner : runner, builder, ...components], {
    env: { ...process.env, BUILD_EVENTS: events }, encoding: 'utf8', timeout: 5000,
  });
  const lines = readFileSync(events, 'utf8').trim().split('\n');
  rmSync(directory, { recursive: true, force: true });
  return { result, lines };
}

test('runs image builds two at a time and waits for every selected component', () => {
  const { result, lines } = run(['api', 'worker', 'cache', 'web']);
  assert.equal(result.status, 0, result.stderr);
  let active = 0;
  let peak = 0;
  for (const line of lines) {
    active += line.startsWith('start ') ? 1 : -1;
    peak = Math.max(peak, active);
    assert.ok(active >= 0 && active <= 2, lines.join(', '));
  }
  assert.equal(peak, 2);
  assert.equal(active, 0);
  assert.equal(lines.filter(line => line.startsWith('end ')).length, 4);
});

test('a failed build prevents later launches and drains active builds', () => {
  const { result, lines } = run(['slow', 'fastfail', 'never']);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /fastfail/);
  assert.ok(lines.includes('end slow'));
  assert.ok(!lines.includes('start never'));
});

test('retains exit status when both children finish before collection', () => {
  const success = run(['api', 'worker', 'cache', 'web'], true);
  assert.equal(success.result.status, 0, success.result.stderr);
  assert.equal(success.lines.filter(line => line.startsWith('end ')).length, 4);
  const failure = run(['slow', 'fastfail', 'never'], true);
  assert.notEqual(failure.result.status, 0);
  assert.match(failure.result.stderr, /Image preparation failed for fastfail/);
  assert.ok(failure.lines.includes('end slow'));
  assert.ok(!failure.lines.includes('start never'));
});

test('refills a completed slot while its slow sibling is still running', () => {
  const { result, lines } = run(['slowlong', 'fast', 'next'], true);
  assert.equal(result.status, 0, result.stderr);
  assert.ok(lines.indexOf('start next') < lines.indexOf('end slowlong'), lines.join(', '));
});
