import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { test } from 'node:test'
import { setTimeout as delay } from 'node:timers/promises'
import { fileURLToPath, pathToFileURL } from 'node:url'

const nitroRequire = createRequire(fileURLToPath(import.meta.resolve('nitro/package.json')))
const { NodeWorkerEnvRunner } = await import(
  pathToFileURL(nitroRequire.resolve('env-runner/runners/node-worker')).href
)

const entry = fileURLToPath(new URL('./fixtures/dev-runner/short-keep-alive-entry.mjs', import.meta.url))
const workerIdleCloseMs = 2000

test('a request after the SSR worker closes an idle connection during a busy dev server tick succeeds', async (t) => {
  const runner = new NodeWorkerEnvRunner({ name: 'connection-reuse', data: { entry } })
  t.after(() => runner.close())
  await runner.waitForReady()

  assert.equal(await (await runner.fetch('http://localhost/')).text(), 'ok')
  await delay(workerIdleCloseMs - 200)
  blockEventLoop(400)

  assert.equal(await (await runner.fetch('http://localhost/')).text(), 'ok')
})

function blockEventLoop(milliseconds) {
  const until = Date.now() + milliseconds
  while (Date.now() < until) {}
}
