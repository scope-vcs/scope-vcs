import { setTimeout as delay } from 'node:timers/promises'
import { serverFunctionName } from './server-functions-smoke.mjs'

export async function delayInitialRepositoryReconciliation(page, milliseconds) {
  let delayed = false
  await page.route('**/_serverFn/**', async (route) => {
    if (delayed || serverFunctionName(route.request()) !== 'loadRepoLiveState_createServerFn_handler') {
      await route.fallback()
      return
    }
    delayed = true
    const response = await route.fetch()
    await delay(milliseconds)
    await route.fulfill({ response })
  })
  return () => delayed
}
