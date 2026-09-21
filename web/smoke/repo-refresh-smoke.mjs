import assert from 'node:assert/strict'
import { setTimeout as delay } from 'node:timers/promises'
import { serverFunctionName } from './server-functions-smoke.mjs'

// Install before navigation. Request-budget tests measure user interactions
// after the initial connection catch-up, not while it is still loading.
export function trackRepositoryRefresh(page) {
  const pending = new Set()
  let summaries = 0
  let lastActivity = Date.now()
  page.on('request', request => {
    if (!request.url().includes('/_serverFn/')) return
    pending.add(request)
    lastActivity = Date.now()
  })
  const finish = request => {
    if (!pending.delete(request)) return
    if (serverFunctionName(request) === 'loadRepoLiveState_createServerFn_handler') summaries++
    lastActivity = Date.now()
  }
  page.on('requestfinished', finish)
  page.on('requestfailed', finish)
  return async () => {
    const deadline = Date.now() + 30_000
    while (!summaries || pending.size || Date.now() - lastActivity < 200) {
      assert(Date.now() < deadline, `initial repository reconciliation did not settle: ${summaries} summaries, pending ${[...pending].map(serverFunctionName).join(', ')}`)
      await delay(50)
    }
    await page.waitForFunction(() => globalThis.__TSR_ROUTER__.state.status === 'idle')
  }
}
