import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import test from 'node:test';
import { execute } from './subprocess.mjs';

async function running(pid) {
  try { return !(await readFile(`/proc/${pid}/stat`, 'utf8')).split(') ')[1].startsWith('Z '); }
  catch (error) { if (error.code === 'ENOENT') return false; throw error; }
}

test('timeout terminates the time wrapper and its child holding inherited pipes', { skip: process.platform !== 'linux' }, async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'scope-subprocess-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const pidPath = join(root, 'pid');
  const started = Date.now();
  const result = await execute('/usr/bin/time', [process.execPath, '-e',
    "require('fs').writeFileSync(process.argv[1], String(process.pid)); setInterval(() => {}, 1000)", pidPath], { timeoutMs: 500 });
  const pid = Number(await readFile(pidPath, 'utf8'));
  assert.equal(result.timedOut, true);
  assert.notEqual(result.code, 0);
  assert.ok(Date.now() - started < 5000);
  for (let attempt = 0; attempt < 30 && await running(pid); attempt++) await delay(10);
  assert.equal(await running(pid), false);
});

test('missing input and broken child stdin become failed results without an unhandled stream error', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'scope-subprocess-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const missing = await execute(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {
    stdinPath: join(root, 'missing'), timeoutMs: 2000,
  });
  assert.match(missing.error, /ENOENT/);
  const input = join(root, 'input');
  await writeFile(input, Buffer.alloc(1024 * 1024));
  const broken = await execute(process.execPath, ['-e', 'process.stdin.destroy(); process.exit(0)'], {
    stdinPath: input, timeoutMs: 2000,
  });
  assert.ok(broken.error || broken.code !== 0);
});
