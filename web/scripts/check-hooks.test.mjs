import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { test } from 'node:test'
import { promisify } from 'node:util'

const run = promisify(execFile)
const cwd = new URL('../', import.meta.url).pathname

test('hooks checks reject order and dependency violations', async () => {
  await assert.rejects(
    run('pnpm', ['exec', 'oxlint', '-c', '.oxlintrc.json', 'scripts/fixtures/hooks/invalid.tsx'], { cwd }),
    (error) => error.code === 1 && /rules-of-hooks/.test(error.stdout) && /exhaustive-deps/.test(error.stdout),
  )
})

test('hooks checks accept a valid component', async () => {
  const { stdout } = await run('pnpm', ['exec', 'oxlint', '-c', '.oxlintrc.json', 'scripts/fixtures/hooks/valid.tsx'], { cwd })
  assert.equal(stdout.trim(), '')
})
