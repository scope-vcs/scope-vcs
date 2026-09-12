import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  assertDocumentPreserved,
  authEnabled,
  baseUrl,
  markDocument,
  repoPath,
  requestRepoPath,
  waitForClientHydration,
  withPage,
} from './browser-smoke.mjs'
import { serverFunctionName } from './server-functions-smoke.mjs'

const holdEventStream = (page) => page.route('**/v1/repos/*/*/events', () => new Promise(() => {}))

test(`sign-in keeps Scope navigation when authentication is ${authEnabled ? 'enabled' : 'disabled'}`, async () => {
  await withPage('/sign-in', async (page) => {
    if (authEnabled) {
      await page.getByRole('textbox', { name: 'Email address', exact: true }).waitFor()
      await page.getByLabel('Password', { exact: true }).waitFor()
      await page.getByRole('link', { name: 'Scope home', exact: true }).click()
    } else {
      await page.getByText('Sign in is disabled in this preview.', { exact: false }).waitFor()
      await page.getByRole('link', { name: 'Back to Scope', exact: true }).click()
    }
    await page.waitForURL(`${baseUrl}/`)
  }, { viewport: { width: 390, height: 844 } })
})

test('changes retry keeps the document and selected revision', async () => {
  let injected = false
  await withPage(`${requestRepoPath}/requests/req_demo_ready`, async (page) => {
    const changes = page.locator('#discussion-discussion_demo_revision_jitter').getByRole('link', { name: /View revision/ })
    await waitForClientHydration(changes)
    await changes.click()
    const retry = page.getByRole('button', { name: 'retry changes', exact: true })
    await retry.waitFor()
    assert.equal(injected, true)
    const before = page.url()
    assert.equal(new URL(before).searchParams.get('revision'), 'event_req_demo_ready_revision_2')
    const heading = await page.getByRole('heading', { name: 'Add bounded retry timing' }).elementHandle()
    await markDocument(page, 'preserved')
    const requests = []
    page.on('request', (request) => {
      if (request.url().includes('/_serverFn/')) {
        requests.push(serverFunctionName(request))
      }
    })
    await waitForClientHydration(retry)
    const response = page.waitForResponse((response) => response.url().includes('/_serverFn/'))
    await retry.click()
    await response
    await retry.waitFor({ state: 'detached' })
    assert.equal(new URL(page.url()).pathname, new URL(before).pathname)
    for (const [key, value] of new URL(before).searchParams) {
      assert.equal(new URL(page.url()).searchParams.get(key), value)
    }
    await assertDocumentPreserved(page, 'preserved')
    assert.equal(await heading.evaluate((element) => element.isConnected), true)
    assert.deepEqual(requests, ['loadChangesPage_createServerFn_handler'])
    await page.getByLabel('Commit file navigator').waitFor()
    assert.equal(await retry.count(), 0)
    const discussion = page.getByRole('navigation', { name: 'Request views' }).getByRole('link', { name: 'Discussion', exact: true })
    await discussion.click()
    await page.waitForURL((url) => url.pathname.endsWith('/req_demo_ready'))
  }, {
    prepare: async (page) => {
      await holdEventStream(page)
      await page.route('**/_serverFn/**', async (route) => {
        const name = serverFunctionName(route.request())
        if (!injected && name === 'loadChangesPage_createServerFn_handler') {
          injected = true
          await route.fulfill({
            contentType: 'application/json',
            body: JSON.stringify({ result: { discussionReferences: { commitKey: null, page: null }, revisions: null }, context: {} }),
          })
        } else {
          await route.continue()
        }
      })
    },
  })
})

test('a delayed file offers scoped retry and keeps its selection', async () => {
  let release = () => {}
  try {
    await withPage(repoPath, async (page) => {
      await page.locator('iframe[title="README.html preview"]').waitFor()
      let attempts = 0
      const held = new Promise((resolve) => { release = resolve })
      await page.route('**/_serverFn/**', async (route) => {
        if (decodeURIComponent(route.request().url()).includes('src/app.ts')) {
          attempts += 1
          if (attempts === 1) await held
        }
        await route.continue().catch(() => {})
      })
      const expand = page.getByRole('button', { name: 'Expand src', exact: true })
      await waitForClientHydration(expand)
      await expand.click()
      await page.getByRole('button', { name: 'app.ts', exact: true }).click()
      await page.getByText('this file is taking longer than usual', { exact: true }).waitFor({ timeout: 30_000 })
      assert.equal(await page.locator('[data-slot="pending-surface"]').getAttribute('aria-busy'), 'true')
      assert.equal(new URL(page.url()).searchParams.get('file'), 'src/app.ts')
      assert.equal(await page.getByLabel('Repository file navigator').isVisible(), true)
      await page.getByRole('button', { name: 'retry file', exact: true }).click()
      await page.locator('pre code').filter({ hasText: 'export function greet' }).waitFor()
      assert.equal(attempts, 2)
      assert.equal(new URL(page.url()).searchParams.get('file'), 'src/app.ts')
    }, { prepare: holdEventStream })
  } finally {
    release()
  }
})
