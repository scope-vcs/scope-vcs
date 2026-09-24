import assert from 'node:assert/strict'
import { test } from 'node:test'
import { assertNoHorizontalOverflow, baseUrl, repo, repoPath, requestRepoPath, waitForClientHydration, withPage } from './browser-smoke.mjs'
import { serverFunctionName } from './server-functions-smoke.mjs'
import { trackRepositoryRefresh } from './repo-refresh-smoke.mjs'

test('latest repository activity survives child navigation without another request or pending state', async () => {
  let requests = 0
  let settled
  const countActivityRequests = (page) => {
    settled = trackRepositoryRefresh(page)
    return page.route('**/_serverFn/**', (route) => {
      if (serverFunctionName(route.request()) === 'loadRepositoryLatestActivity_createServerFn_handler') requests += 1
      return route.continue()
    })
  }
  await withPage(repoPath, async (page) => {
    await settled()
    const activity = page.getByLabel('Latest repository change', { exact: true })
    await activity.waitFor()
    const original = await activity.innerText()
    const firstRequests = requests
    assert.equal(firstRequests, 2) // Initial read and connection catch-up.
    for (const width of [1280, 390]) {
      await page.setViewportSize({ width, height: 844 })
      await page.getByRole('link', { name: 'Requests', exact: true }).first().click()
      await page.waitForURL(`**/${repo}/requests`)
      await page.getByRole('link', { name: 'Code', exact: true }).first().click()
      await page.waitForURL(`${baseUrl}${repoPath}`)
      assert.equal(await activity.isVisible(), true)
      assert.equal(await activity.innerText(), original)
      assert.equal(await page.getByLabel('Loading latest repository change', { exact: true }).count(), 0)
      assert.equal(requests, firstRequests)
      await assertNoHorizontalOverflow(page)
    }
  }, { prepare: countActivityRequests, viewport: { width: 1280, height: 900 } })
})

test('request queue and summary are reused across actual child-route navigation', async () => {
  let settled
  await withPage(`${requestRepoPath}/requests`, async page => {
    await settled()
    const reads = []
    page.on('request', request => {
      const name = serverFunctionName(request)
      if (['loadRepoLiveState_createServerFn_handler', 'loadRequestQueuePage_createServerFn_handler'].includes(name)) reads.push(name)
    })
    await page.getByRole('link', { name: 'Code', exact: true }).first().click()
    await page.waitForURL(`${baseUrl}${requestRepoPath}`)
    await page.getByRole('link', { name: 'Requests', exact: true }).first().click()
    await page.waitForURL(`${baseUrl}${requestRepoPath}/requests`)
    await page.getByRole('link', { name: /Add bounded retry timing/ }).waitFor()
    await page.waitForFunction(() => globalThis.__TSR_ROUTER__.state.status === 'idle')
    assert.deepEqual(reads, [])
  }, { prepare: page => { settled = trackRepositoryRefresh(page) } })
})

test('repository events received off-page refresh retained activity without blanking it', async () => {
  let requests = 0
  let originalMessage = ''
  let holdActivityRefresh = false
  let settled
  let release
  const held = new Promise((resolve) => { release = resolve })
  const prepare = async (page) => {
    settled = trackRepositoryRefresh(page)
    await page.addInitScript(() => {
      const originalFetch = window.fetch.bind(window)
      const streams = new Set()
      window.__scopeRepositoryStreamCount = () => streams.size
      window.__scopeEmitRepositoryEvent = (event) => {
        for (const stream of streams) stream.enqueue(new TextEncoder().encode(`event: repo-change\ndata: ${JSON.stringify(event)}\n\n`))
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
          },
        })
        return Promise.resolve(new Response(body, { headers: { 'content-type': 'text/event-stream' } }))
      }
    })
    await page.route('**/_serverFn/**', async (route) => {
      if (serverFunctionName(route.request()) !== 'loadRepositoryLatestActivity_createServerFn_handler') {
        await route.continue()
        return
      }
      requests += 1
      const hold = holdActivityRefresh
      const response = await route.fetch()
      if (!hold) {
        await route.fulfill({ response })
        return
      }
      const body = await response.text()
      assert(body.includes(JSON.stringify(originalMessage)))
      const updated = body.replace(JSON.stringify(originalMessage), JSON.stringify('New repository activity'))
      await held
      await route.fulfill({ response, body: updated })
    })
  }
  try {
    await withPage(repoPath, async (page) => {
      const emitEvent = (kind) => page.evaluate((kind) => {
        const repo = globalThis.__TSR_ROUTER__.state.matches.find((match) => match.loaderData?.repo)?.loaderData.repo
        if (!repo) throw new Error('Repository layout is unavailable')
        window.__scopeEmitRepositoryEvent({ repo_id: repo.id, incarnation_id: 'browser-test', version: 1, kind })
      }, kind)
      await page.waitForFunction(() => globalThis.__TSR_ROUTER__?.state.status === 'idle' && window.__scopeRepositoryStreamCount() > 0)
      // A real stream starts with Connected. Let its catch-up finish before
      // holding the later repository change, including any cancelled reads.
      await emitEvent('Connected')
      await settled()
      const activity = page.getByLabel('Latest repository change', { exact: true })
      await activity.waitFor()
      const original = await activity.innerText()
      originalMessage = await activity.getByRole('link').first().innerText()
      const initialRequests = requests
      holdActivityRefresh = true
      await page.getByRole('link', { name: 'Requests', exact: true }).first().click()
      await page.waitForURL(`**/${repo}/requests`)
      await page.waitForFunction(() => globalThis.__TSR_ROUTER__.state.status === 'idle' && window.__scopeRepositoryStreamCount() > 0)
      const revalidated = page.waitForResponse((response) => serverFunctionName(response.request()) === 'loadRepoLiveState_createServerFn_handler')
      await emitEvent({ RepositoryChanged: { reason: 'push' } })
      await (await revalidated).finished()
      await page.waitForFunction(() => globalThis.__TSR_ROUTER__.state.status === 'idle')
      await page.getByRole('link', { name: 'Code', exact: true }).first().click()
      await page.waitForURL(`${baseUrl}${repoPath}`)
      assert.equal(await activity.isVisible(), true)
      assert.equal(await activity.innerText(), original)
      assert.equal(await page.getByLabel('Loading latest repository change', { exact: true }).count(), 0)
      release()
      await activity.getByRole('link', { name: 'New repository activity', exact: true }).waitFor()
      assert.equal(requests, initialRequests + 1)
    }, { prepare })
  } finally {
    release()
  }
})

test('leaving and returning during a file load reuses its pending resource request', async () => {
  let requests = 0
  let settled
  let holdRequestedFile = false
  let release
  const held = new Promise((resolve) => { release = resolve })
  const holdFile = (page) => {
    settled = trackRepositoryRefresh(page)
    return page.route('**/_serverFn/**', async (route) => {
      if (holdRequestedFile && serverFunctionName(route.request()) === 'loadRepoFile_createServerFn_handler') {
        requests += 1
        await held
      }
      await route.continue()
    })
  }
  try {
    await withPage(repoPath, async (page) => {
      await settled()
      holdRequestedFile = true
      await waitForClientHydration(page.getByRole('button', { name: 'Switch to light mode' }))
      await page.evaluate((to) => { void globalThis.__TSR_ROUTER__.navigate({ to, search: { file: 'src/app.ts' } }) }, repoPath)
      await page.getByRole('tab', { name: 'src/app.ts', exact: true }).waitFor()
      await page.getByRole('link', { name: 'Requests', exact: true }).first().click()
      await page.waitForURL(`**/${repo}/requests`)
      await page.goBack()
      await page.getByRole('tab', { name: 'src/app.ts', exact: true }).waitFor()
      release()
      await page.locator('pre code').filter({ hasText: 'export function greet' }).waitFor()
      assert.equal(requests, 1)
      assert.equal(await page.getByRole('alert').count(), 0)
    }, { prepare: holdFile })
  } finally {
    release()
  }
})
