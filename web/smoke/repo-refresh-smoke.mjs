import { setTimeout as delay } from 'node:timers/promises'
import { serverFunctionName } from './server-functions-smoke.mjs'

export async function controlRepositoryEvents(page, { initialConnected = false } = {}) {
  await page.addInitScript((initialConnected) => {
    const originalFetch = window.fetch.bind(window)
    const streams = new Set()
    window.__scopeRepositoryStreamCount = () => streams.size
    const message = (kind) => {
      const repo = globalThis.__TSR_ROUTER__.state.matches.find((match) => match.loaderData?.repo)?.loaderData.repo
      if (!repo) throw new Error('Repository layout is unavailable')
      const event = { repo_id: repo.id, incarnation_id: 'browser-test', version: 1, kind }
      return new TextEncoder().encode(`event: repo-change\ndata: ${JSON.stringify(event)}\n\n`)
    }
    window.__scopeEmitRepositoryEvent = (kind) => {
      for (const stream of streams) stream.enqueue(message(kind))
    }
    window.fetch = (input, init) => {
      const url = new URL(typeof input === 'string' ? input : input.url ?? String(input), location.href)
      if (!url.pathname.endsWith('/events')) return originalFetch(input, init)
      const body = new ReadableStream({
        start(controller) {
          streams.add(controller)
          init?.signal?.addEventListener('abort', () => {
            streams.delete(controller)
            controller.close()
          }, { once: true })
          if (initialConnected) controller.enqueue(message('Connected'))
        },
      })
      return Promise.resolve(new Response(body, { headers: { 'content-type': 'text/event-stream' } }))
    }
  }, initialConnected)
  return (kind) => page.evaluate((kind) => {
    window.__scopeEmitRepositoryEvent(kind)
  }, kind)
}

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
