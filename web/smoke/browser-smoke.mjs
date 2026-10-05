import assert from 'node:assert/strict'
import { setTimeout as delay } from 'node:timers/promises'
import { chromium } from 'playwright'
import { serverFunctionName } from './server-functions-smoke.mjs'

export const baseUrl = (process.env.SCOPE_WEB_BASE_URL ?? 'http://localhost:3000').replace(/\/$/, '')

const repoId = process.env.SCOPE_SMOKE_REPO ?? 'dev/public-demo'
const [owner, repoName, extra] = repoId.split('/')
if (!owner || !repoName || extra) {
  throw new Error('SCOPE_SMOKE_REPO must be an owner/repository pair')
}

export { owner }
export const repo = `${owner}/${repoName}`
export const repoPath = `/${repo}`
export const requestRepoPath = `/${owner}/update-demo`
export const authEnabled = process.env.SCOPE_SMOKE_AUTH_ENABLED === '1'
const serverFunctionLatencyMs = Number(process.env.SCOPE_SMOKE_LATENCY_MS ?? 0)

export async function withBlankPage(run, pageOptions = {}) {
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage(pageOptions)
  const pageErrors = []
  page.on('pageerror', (error) => pageErrors.push(error.message))
  try {
    await page.addInitScript(() => {
      globalThis.customElements?.define('vite-error-overlay', class extends HTMLElement {
        connectedCallback() { this.remove() }
      })
    })
    if (serverFunctionLatencyMs > 0) {
      const session = await page.context().newCDPSession(page)
      await session.send('Network.enable')
      await session.send('Network.emulateNetworkConditionsByRule', {
        matchedNetworkConditions: [{
          urlPattern: '*://*:*/_serverFn/*',
          latency: serverFunctionLatencyMs,
          downloadThroughput: -1,
          uploadThroughput: -1,
        }],
      })
    }
    await run(page)
    assert.deepEqual(pageErrors, [])
  } finally {
    await browser.close()
  }
}

export async function withPage(path, run, { prepare, settle = true, ...pageOptions } = {}) {
  await withBlankPage(async (page) => {
    const settled = settle && pageOptions.javaScriptEnabled !== false ? trackPageSettle(page) : null
    await prepare?.(page)
    const response = await page.goto(new URL(path, `${baseUrl}/`).toString(), {
      timeout: 30_000,
      waitUntil: 'domcontentloaded',
    })
    assert(response, `navigation to ${path} did not produce a response`)
    assert(response.status() < 400, `navigation to ${path} returned ${response.status()}`)
    await settled?.()
    await run(page)
  }, pageOptions)
}

const SERVER_FUNCTION_QUIET_MS = 200

export function trackPageSettle(page) {
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
    await page.waitForFunction(() => Boolean(globalThis.__TSR_ROUTER__?.state.resolvedLocation))
    const repository = await page.evaluate(() =>
      globalThis.__TSR_ROUTER__.state.matches.some(({ routeId }) => routeId === '/$owner/$repo'))
    const deadline = Date.now() + 30_000
    while ((repository && !summaries) || pending.size || Date.now() - lastActivity < SERVER_FUNCTION_QUIET_MS) {
      assert(Date.now() < deadline, `page did not settle: ${summaries} live-state summaries, pending ${[...pending].map(serverFunctionName).join(', ')}`)
      await delay(50)
    }
    await page.waitForFunction(() => globalThis.__TSR_ROUTER__.state.status === 'idle')
  }
}

async function isClientHydrated(locator) {
  return locator.evaluate((element) => Object.keys(element).some((key) => key.startsWith('__reactProps$')))
}

export async function waitForClientHydration(locator) {
  const deadline = Date.now() + 30_000
  while (!await isClientHydrated(locator)) {
    assert(Date.now() < deadline, 'element did not hydrate within 30 seconds')
    await delay(50)
  }
}

export async function markDocument(page, sentinel) {
  await page.evaluate((value) => { window.__scopeSmokeDocument = value }, sentinel)
}

export async function assertDocumentPreserved(page, sentinel) {
  assert.equal(await page.evaluate(() => window.__scopeSmokeDocument), sentinel)
}

export async function captureNodes(page, selectors) {
  const nodes = {}
  for (const selector of selectors) nodes[selector] = await page.locator(selector).elementHandle()
  return nodes
}

export async function assertNodesPreserved(page, nodes) {
  assert.equal(
    await page.evaluate(
      (entries) => entries.every(([selector, node]) => node === document.querySelector(selector)),
      Object.entries(nodes),
    ),
    true,
  )
}

export function captureRepositoryChrome(page) {
  return captureNodes(page, ['header.sticky', 'nav[aria-label="Primary"]'])
}

export async function within(promise, timeoutMs, message) {
  let timeout
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timeout = setTimeout(() => reject(new Error(message)), timeoutMs)
      }),
    ])
  } finally {
    clearTimeout(timeout)
  }
}

export async function assertCurrentRepoSection(page, section) {
  const link = page
    .getByRole('navigation', { name: 'Primary' })
    .getByRole('link', { name: section, exact: true })
  await link.waitFor()
  assert.equal(await link.getAttribute('aria-current'), 'page')
  await page.waitForFunction(
    (expected) => {
      const current = [
        ...document.querySelectorAll(
          'nav[aria-label="Primary"] a[aria-current="page"]',
        ),
      ].map((link) => link.getAttribute('href'))
      return current.length === 1 && current[0] === expected
    },
    await link.getAttribute('href'),
  )
}

export async function assertPageHeading(page, title) {
  await page.getByRole('heading', { level: 1, name: title }).waitFor({
    state: 'attached',
  })
}

export async function assertPassiveSkeleton(page, selector) {
  const skeletons = page.locator(`${selector} [data-slot="skeleton"]:visible`)
  await skeletons.first().waitFor()
  assert.equal((await skeletons.count()) > 0, true)
  assert.deepEqual(
    await skeletons.evaluateAll((elements) =>
      elements.map((element) => element.getAttribute('aria-hidden')),
    ),
    Array.from({ length: await skeletons.count() }, () => 'true'),
  )
  assert.equal(
    await page.locator(`${selector} .animate-spin:visible`).count(),
    0,
  )
  assert.deepEqual(
    await page.locator(selector).evaluate((root) => {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
      const visibleLoadingText = []
      let node = walker.nextNode()
      while (node) {
        const text = node.textContent?.trim() ?? ''
        const parent = node.parentElement
        if (
          /^(Loading|Connecting)\b/i.test(text) &&
          parent &&
          !parent.closest('.sr-only') &&
          parent.getClientRects().length > 0
        ) {
          visibleLoadingText.push(text)
        }
        node = walker.nextNode()
      }
      return visibleLoadingText
    }),
    [],
  )
}

export async function assertNoHorizontalOverflow(page) {
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
}

export async function assertMobileFilesCollapsed(page, selectedFile) {
  const toggle = page.getByRole('button', { name: `files ${selectedFile}`, exact: true })
  assert.equal(await toggle.getAttribute('aria-expanded'), 'false')
  await assertNoHorizontalOverflow(page)
}
