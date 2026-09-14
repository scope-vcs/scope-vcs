import assert from 'node:assert/strict';
import { chmod, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { execute } from './subprocess.mjs';

test('telemetry CLI uses the deployment services, explicit overrides, and exact requested time window', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'scope-telemetry-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const manifest = JSON.parse(await readFile(new URL('../.github/deployment-services.json', import.meta.url), 'utf8'));
  const names = ['api', 'run-worker'].map((key) => manifest.services[key].name);
  const executable = join(root, 'railway');
  await writeFile(executable, `#!${process.execPath}
const args = process.argv.slice(2);
const name = args[args.indexOf('--service') + 1];
if (!${JSON.stringify([...names, 'override-worker'])}.includes(name)) process.exit(2);
for (const [flag, expected] of [['--since', '2026-08-22T20:00:00Z'], ['--until', '2026-08-22T20:05:00Z']]) {
  if (args[args.indexOf(flag) + 1] !== expected) process.exit(3);
}
if (args[0] === 'metrics') console.log(JSON.stringify({ measurements: {} }));
`);
  await chmod(executable, 0o700);
  for (const override of ['', 'override-worker']) {
    const output = join(root, override || 'default');
    await mkdir(output);
    const result = await execute(process.execPath, [fileURLToPath(new URL('./railway-telemetry.mjs', import.meta.url))], {
      cwd: root, captureStdout: true, timeoutMs: 10000,
      env: { PATH: `${root}:${process.env.PATH}`, SCOPE_RAILWAY_ENVIRONMENT: 'test',
        SCOPE_RAILWAY_SERVICES: override, SCOPE_RAILWAY_TELEMETRY_DIR: output,
        SCOPE_RAILWAY_SINCE: '2026-08-22T20:00:00Z', SCOPE_RAILWAY_UNTIL: '2026-08-22T20:05:00Z' },
    });
    assert.equal(result.code, 0, result.stderr);
    const files = await readdir(output);
    const report = JSON.parse(await readFile(join(output, files.find((file) => file.endsWith('.json'))), 'utf8'));
    assert.deepEqual(Object.keys(report.services), override ? [override] : names);
    assert.equal(files.filter((file) => file.endsWith('.md')).length, 1);
  }
});
