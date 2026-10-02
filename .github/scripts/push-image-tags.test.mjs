import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

const script = new URL('./push-image-tags.sh', import.meta.url).pathname;

function push(t, failures) {
  const root = mkdtempSync(join(tmpdir(), 'scope-image-push-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  writeFileSync(join(root, 'sleep'), '#!/bin/sh\nexit 0\n', { mode: 0o755 });
  writeFileSync(join(root, 'docker'), `#!/bin/bash
set -euo pipefail
[[ "$1" == push ]]
printf '%s\\n' "$2" >> "$TEST_PUSHES"
attempts="$(grep -cx -- "$2" "$TEST_PUSHES")"
(( attempts > TEST_FAILURES )) || { echo 'Get "https://ghcr.io/v2/": context deadline exceeded' >&2; exit 1; }
`, { mode: 0o755 });
  const result = spawnSync('bash', [script], {
    input: 'ghcr.io/scope/checks:sha-a\nghcr.io/scope/checks:main\n',
    encoding: 'utf8', timeout: 10_000,
    env: { ...process.env, PATH: `${root}:${process.env.PATH}`, TEST_PUSHES: join(root, 'pushes'), TEST_FAILURES: String(failures) },
  });
  return { result, pushes: readFileSync(join(root, 'pushes'), 'utf8').trim().split('\n') };
}

test('image push retries a transient registry failure for each tag', t => {
  const { result, pushes } = push(t, 1);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(pushes, [
    'ghcr.io/scope/checks:sha-a', 'ghcr.io/scope/checks:sha-a',
    'ghcr.io/scope/checks:main', 'ghcr.io/scope/checks:main',
  ]);
});

test('image push fails after three attempts without pushing later tags', t => {
  const { result, pushes } = push(t, 3);
  assert.notEqual(result.status, 0);
  assert.deepEqual(pushes, Array(3).fill('ghcr.io/scope/checks:sha-a'));
});
