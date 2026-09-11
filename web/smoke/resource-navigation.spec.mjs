import assert from 'node:assert/strict'
import { test } from 'node:test'
import { chromium } from 'playwright'
import { serverFunctionName } from './server-functions-smoke.mjs'

const baseUrl = process.env.SCOPE_WEB_BASE_URL ?? process.env.PLAYWRIGHT_BASE_URL ?? 'http://localhost:3000'
const repo = process.env.SCOPE_SMOKE_REPO ?? 'dev/public-demo'

test('latest repository activity survives child navigation without another request or pending state', async () => {
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
  let requests = 0
  const pageErrors = []
  page.on('pageerror', (error) => pageErrors.push(error.message))
  await page.route('**/_serverFn/**', (route) => {
    if (serverFunctionName(route.request()) === 'loadRepositoryLatestActivity_createServerFn_handler') requests += 1
    return route.continue()
  })
  try {
    await page.goto(`${baseUrl}/${repo}`)
    const activity = page.getByLabel('Latest repository change', { exact: true })
    await activity.waitFor()
    const original = await activity.innerText()
    const firstRequests = requests
    assert.equal(firstRequests, 1)
    for (const width of [1280, 390]) {
      await page.setViewportSize({ width, height: 844 })
      await page.getByRole('link', { name: 'Requests', exact: true }).first().click()
      await page.waitForURL(`**/${repo}/requests`)
      await page.getByRole('link', { name: 'Code', exact: true }).first().click()
      await page.waitForURL(`${baseUrl}/${repo}`)
      assert.equal(await activity.isVisible(), true)
      assert.equal(await activity.innerText(), original)
      assert.equal(await page.getByLabel('Loading latest repository change', { exact: true }).count(), 0)
      assert.equal(requests, firstRequests)
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
    }
    assert.deepEqual(pageErrors, [])
  } finally {
    await browser.close()
  }
})

test('repository events refresh retained activity and public source off-page without blanking either', async () => {
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage()
  let requests = 0
  let files = 0
  let trees = 0
  let originalMessage = ''
  let release
  const held = new Promise((resolve) => { release = resolve })
  await installRepositoryStream(page)
  await page.route('**/_serverFn/**', async (route) => {
    const name = serverFunctionName(route.request())
    if (name === 'loadRepoContent_createServerFn_handler') trees += 1
    if (name === 'loadRepoFile_createServerFn_handler' && ++files > 1) {
      const response = await route.fetch()
      const body = await response.text()
      assert(body.includes('export function greet'))
      await held
      return route.fulfill({ response, body: body.replaceAll('export function greet', 'export function refreshedGreet') })
    }
    if (name !== 'loadRepositoryLatestActivity_createServerFn_handler') {
      await route.continue()
      return
    }
    requests += 1
    const response = await route.fetch()
    if (requests === 1) {
      await route.fulfill({ response })
      return
    }
    const body = await response.text()
    assert(body.includes(JSON.stringify(originalMessage)))
    const updated = body.replace(JSON.stringify(originalMessage), JSON.stringify('New repository activity'))
    await held
    await route.fulfill({ response, body: updated })
  })
  try {
    await page.goto(`${baseUrl}/${repo}?file=src%2Fapp.ts`)
    const source = page.locator('pre code').filter({ hasText: 'export function greet' })
    await source.waitFor()
    assert.equal(files, 1)
    assert.equal(trees, 1)
    const activity = page.getByLabel('Latest repository change', { exact: true })
    await activity.waitFor()
    const original = await activity.innerText()
    originalMessage = await activity.getByRole('link').first().innerText()
    await page.getByRole('link', { name: 'Requests', exact: true }).first().click()
    await page.waitForURL(`**/${repo}/requests`)
    await page.waitForFunction(() => globalThis.__TSR_ROUTER__.state.status === 'idle' && window.__scopeRepositoryStreamCount() > 0)
    const revalidated = page.waitForResponse((response) => serverFunctionName(response.request()) === 'loadRepoLiveState_createServerFn_handler')
    await page.evaluate(() => {
      const repo = globalThis.__TSR_ROUTER__.state.matches.find((match) => match.loaderData?.repo)?.loaderData.repo
      if (!repo) throw new Error('Repository layout is unavailable')
      window.__scopeEmitRepositoryEvent({ repo_id: repo.id, incarnation_id: 'browser-test', version: 1, kind: { RepositoryChanged: { reason: 'push' } } })
    })
    await (await revalidated).finished()
    await page.waitForFunction(() => globalThis.__TSR_ROUTER__.state.status === 'idle')
    await page.goBack()
    await source.waitFor()
    assert.equal(await page.getByLabel('Loading file content', { exact: true }).count(), 0)
    assert.equal(await activity.isVisible(), true)
    assert.equal(await activity.innerText(), original)
    assert.equal(await page.getByLabel('Loading latest repository change', { exact: true }).count(), 0)
    release()
    await activity.getByRole('link', { name: 'New repository activity', exact: true }).waitFor()
    await page.locator('pre code').filter({ hasText: 'export function refreshedGreet' }).waitFor()
    assert.equal(requests, 2)
    assert.equal(files, 2)
    assert.equal(trees, 2)
  } finally {
    release()
    await browser.close()
  }
})

test('leaving and returning during a file load reuses its pending resource request', async () => {
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage()
  let requests = 0
  let release
  const held = new Promise((resolve) => { release = resolve })
  await page.route('**/_serverFn/**', async (route) => {
    if (serverFunctionName(route.request()) === 'loadRepoFile_createServerFn_handler') {
      requests += 1
      await held
    }
    await route.continue()
  })
  try {
    await page.goto(baseUrl)
    await page.waitForFunction(() => globalThis.__TSR_ROUTER__)
    await page.evaluate((repo) => { void globalThis.__TSR_ROUTER__.navigate({ to: `/${repo}`, search: { file: 'src/app.ts' } }) }, repo)
    await page.getByRole('tab', { name: 'src/app.ts', exact: true }).waitFor()
    await page.getByRole('link', { name: 'Requests', exact: true }).first().click()
    await page.waitForURL(`**/${repo}/requests`)
    await page.goBack()
    await page.getByRole('tab', { name: 'src/app.ts', exact: true }).waitFor()
    release()
    await page.locator('pre code').filter({ hasText: 'export function greet' }).waitFor()
    assert.equal(requests, 1)
    assert.equal(await page.getByRole('alert').count(), 0)
  } finally {
    release()
    await browser.close()
  }
})

async function installRepositoryStream(page) {
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
}
