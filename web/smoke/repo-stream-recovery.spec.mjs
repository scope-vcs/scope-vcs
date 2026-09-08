import assert from 'node:assert/strict'
import { test } from 'node:test'
import { chromium } from 'playwright'
import { serverFunctionName } from './server-functions-smoke.mjs'

const baseUrl = process.env.SCOPE_WEB_BASE_URL ?? 'http://localhost:3000'
const repo = process.env.SCOPE_SMOKE_REPO ?? 'dev/public-demo'

test('an interrupted stream recovers through failed reloads without replacing repository data', async () => {
  const browser = await chromium.launch({ headless: true })
  try {
    const page = await browser.newPage()
    let interrupted = false
    let failedReloads = 0
    let streamRequests = 0
    page.on('request', (request) => {
      if (new URL(request.url()).pathname.endsWith(`/v1/repos/${repo}/events`)) streamRequests += 1
    })
    await page.addInitScript(() => {
      const originalFetch = window.fetch.bind(window)
      window.fetch = (input, init) => {
        const url = typeof input === 'string' ? input : input.url ?? String(input)
        if (!new URL(url, location.href).pathname.endsWith('/events')) return originalFetch(input, init)
        const controller = new AbortController()
        init?.signal?.addEventListener('abort', () => controller.abort(), { once: true })
        window.__interruptRepoStream = () => controller.abort()
        return originalFetch(input, { ...init, signal: controller.signal })
      }
    })
    await page.route('**/_serverFn/**', async (route) => {
      if (interrupted && failedReloads < 2 && serverFunctionName(route.request()) === 'loadRepoLiveState_createServerFn_handler') {
        failedReloads += 1
        return route.fulfill({ status: 503, contentType: 'text/plain', body: 'Injected temporary outage' })
      }
      return route.continue()
    })
    await page.goto(`${baseUrl}/${repo}`)
    const activity = page.getByLabel('Latest repository change', { exact: true })
    const navigator = page.getByLabel('Repository file navigator', { exact: true })
    await activity.waitFor()
    await navigator.waitFor()
    await page.getByRole('tabpanel').waitFor()
    await page.waitForFunction(() => globalThis.__TSR_ROUTER__?.state.status === 'idle' && window.__interruptRepoStream)
    const initialActivity = await activity.innerText()
    const initialContent = await page.getByRole('tabpanel').innerText()
    const originalNavigator = await navigator.elementHandle()
    await page.evaluate(() => {
      window.__repoRecoveryBlanked = false
      window.__repoRecoveryObserver = new MutationObserver(() => {
        if (!document.querySelector('[aria-label="Repository file navigator"]') ||
            !document.querySelector('[aria-label="Latest repository change"]')?.textContent?.trim()) {
          window.__repoRecoveryBlanked = true
        }
      })
      window.__repoRecoveryObserver.observe(document.body, { childList: true, subtree: true })
    })
    interrupted = true
    await page.evaluate(() => window.__interruptRepoStream())
    await page.waitForResponse((response) =>
      serverFunctionName(response.request()) === 'loadRepoLiveState_createServerFn_handler' && response.status() === 200,
    { timeout: 15_000 })
    await page.waitForFunction(() => globalThis.__TSR_ROUTER__?.state.status === 'idle')
    assert.equal(failedReloads, 2)
    assert.ok(streamRequests >= 2, 'the event stream must reconnect')
    assert.equal(await originalNavigator.evaluate((node) => node.isConnected), true)
    assert.equal(await activity.innerText(), initialActivity)
    assert.equal(await page.getByRole('tabpanel').innerText(), initialContent)
    assert.equal(await page.evaluate(() => {
      window.__repoRecoveryObserver.disconnect()
      return window.__repoRecoveryBlanked
    }), false)
  } finally {
    await browser.close()
  }
})
