import assert from 'node:assert/strict'
import { chmod, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import { afterEach, test } from 'node:test'

const temporaryDirectories = []

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((path) => rm(path, { force: true, recursive: true })))
})

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), 'scope-staging-stop-'))
  temporaryDirectories.push(root)
  const manifest = join(root, 'manifest.json')
  const removals = join(root, 'removals.jsonl')
  await writeFile(manifest, JSON.stringify({
    railway: {
      projectId: 'project',
    },
    environments: {
      production: { environmentId: 'production' },
      staging: { environmentId: 'staging' },
    },
    services: {
      api: { id: 'api' },
      cache: { id: 'cache' },
      'media-api': { id: 'media' },
      'media-worker': { id: 'media-worker' },
      'run-worker': { id: 'worker' },
    },
  }))
  await writeFile(join(root, 'railway'), `#!/usr/bin/env bash
set -euo pipefail
if [[ "$*" == *"deployment list"* ]]; then
  if [[ "\${SCOPE_TEST_SCENARIO:-}" == "list-fails" ]]; then
    printf '%s\\n' '[]'
    exit 1
  fi
  if [[ "\${SCOPE_TEST_SCENARIO:-}" == "remove-ambiguous" && -f "$SCOPE_TEST_REMOVALS" && "$*" == *"--service api"* ]]; then
    printf '%s\\n' '[{"id":"api-deployment","status":"REMOVED"}]'
    exit 0
  fi
  if [[ "$*" == *"--service api"* ]]; then
    printf '%s\\n' '[{"id":"api-deployment","status":"SUCCESS"}]'
  elif [[ "$*" == *"--service cache"* ]]; then
    printf '%s\\n' '[{"id":"cache-deployment","status":"SUCCESS"}]'
  elif [[ "$*" == *"--service media-worker"* ]]; then
    printf '%s\\n' '[{"id":"media-worker-deployment","status":"SUCCESS"}]'
  elif [[ "$*" == *"--service media"* ]]; then
    printf '%s\\n' '[{"id":"media-deployment","status":"SUCCESS"}]'
  else
    printf '%s\\n' '[{"id":"worker-deployment","status":"SUCCESS"}]'
  fi
elif [[ "$*" == *"service list"* ]]; then
  printf '%s\\n' '[{"id":"api","replicas":{"running":0,"crashed":0}},{"id":"cache","replicas":{"running":0,"crashed":0}},{"id":"worker","replicas":{"running":0,"crashed":0}},{"id":"media","replicas":{"running":0,"crashed":0}},{"id":"media-worker","replicas":{"running":0,"crashed":0}}]'
else
  exit 2
fi
`)
  await writeFile(join(root, 'curl'), `#!/usr/bin/env bash
set -euo pipefail
while [[ "$#" -gt 0 ]]; do
  if [[ "$1" == "--data-binary" ]]; then
    printf '%s\\n' "$2" >> "$SCOPE_TEST_REMOVALS"
    break
  fi
  shift
done
if [[ "\${SCOPE_TEST_SCENARIO:-}" == "remove-false" ]]; then
  printf '%s\\n' '{"data":{"deploymentRemove":false}}'
  exit 0
fi
if [[ "\${SCOPE_TEST_SCENARIO:-}" == "remove-ambiguous" && "$(wc -l < "$SCOPE_TEST_REMOVALS")" == 1 ]]; then
  printf '%s\\n' '{"data":{"deploymentRemove":true}}'
  exit 22
fi
printf '%s\\n' '{"data":{"deploymentRemove":true}}'
`)
  await writeFile(join(root, 'sleep'), '#!/bin/sh\nexit 0\n', { mode: 0o755 })
  await chmod(join(root, 'railway'), 0o755)
  await chmod(join(root, 'curl'), 0o755)
  return { manifest, removals, root }
}

test('stops only the reviewed staging metadata-writer deployments', async () => {
  const { manifest, removals, root } = await fixture()
  const result = spawnSync('bash', ['.github/scripts/stop-staging-writers.sh'], {
    cwd: process.cwd(),
    encoding: 'utf8',
    env: {
      ...process.env,
      PATH: `${root}:${process.env.PATH}`,
      RAILWAY_API_TOKEN: 'account-token',
      RAILWAY_TOKEN: '',
      SCOPE_DEPLOYMENT_MANIFEST: manifest,
      SCOPE_TEST_REMOVALS: removals,
    },
  })

  assert.equal(result.status, 0, result.stderr)
  const requests = (await readFile(removals, 'utf8')).trim().split('\n').map(JSON.parse)
  assert.deepEqual(requests.map(({ variables }) => variables.id), [
    'api-deployment',
    'worker-deployment',
    'cache-deployment',
    'media-deployment',
    'media-worker-deployment',
  ])
})

test('rejects mixed Railway token privileges before making requests', async () => {
  const { manifest, root } = await fixture()
  const result = spawnSync('bash', ['.github/scripts/stop-staging-writers.sh'], {
    cwd: process.cwd(),
    encoding: 'utf8',
    env: {
      ...process.env,
      PATH: `${root}:${process.env.PATH}`,
      RAILWAY_API_TOKEN: 'account-token',
      RAILWAY_TOKEN: 'project-token',
      SCOPE_DEPLOYMENT_MANIFEST: manifest,
    },
  })

  assert.equal(result.status, 1)
  assert.match(result.stderr, /requires only RAILWAY_API_TOKEN/)
})

for (const scenario of ['list-fails', 'remove-false', 'remove-ambiguous']) {
  test(`reconciles staging shutdown failure: ${scenario}`, async () => {
    const { manifest, removals, root } = await fixture()
    const result = spawnSync('bash', ['.github/scripts/stop-staging-writers.sh'], {
      encoding: 'utf8', timeout: 15_000,
      env: {
        ...process.env, PATH: `${root}:${process.env.PATH}`,
        RAILWAY_API_TOKEN: 'account-token', RAILWAY_TOKEN: '',
        SCOPE_DEPLOYMENT_MANIFEST: manifest, SCOPE_TEST_REMOVALS: removals,
        SCOPE_TEST_SCENARIO: scenario,
      },
    })
    assert.equal(result.status, scenario === 'remove-ambiguous' ? 0 : 1, result.stderr)
    if (scenario === 'list-fails') {
      await assert.rejects(readFile(removals), { code: 'ENOENT' })
    } else {
      const calls = (await readFile(removals, 'utf8')).trim().split('\n').map(JSON.parse)
      assert.equal(calls.filter(({ variables }) => variables.id === 'api-deployment').length,
        scenario === 'remove-ambiguous' ? 1 : 3)
    }
  })
}
