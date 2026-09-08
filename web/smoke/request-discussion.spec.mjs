import assert from 'node:assert/strict'
import { test } from 'node:test'
import { chromium } from 'playwright'
import {
  assertFileSelectionSkipsRevisionReload,
  assertRequestCrossLinksStayInDocument,
  assertRequestShellPreserved,
  assertUpdateSelectionReloadsSelectedPayload,
  waitForClientHydration,
} from './request-changes-smoke.mjs'

const baseUrl = (
  process.env.SCOPE_WEB_BASE_URL ??
  process.env.PLAYWRIGHT_BASE_URL ??
  'http://localhost:3000'
).replace(/\/$/, '')
const repoId = process.env.SCOPE_SMOKE_REPO ?? 'dev/public-demo'
const [owner, repo, extra] = repoId.split('/')

if (!owner || !repo || extra) {
  throw new Error('SCOPE_SMOKE_REPO must be an owner/repository pair')
}

test('seeded request discussion and changes stay reciprocal and ordered', async () => {
  await withPage(`/${owner}/update-demo/requests/req_demo_ready`, async (page) => {
    await page.getByRole('heading', { level: 1, name: 'Add bounded retry timing' }).waitFor()
    assert.equal(
      await page.getByRole('button', { name: 'Refresh', exact: true }).count(),
      0,
    )
    await page.getByText('Public request', { exact: true }).last().waitFor()
    const threads = page.locator('.request-discussion-thread')
    await threads.first().waitFor()
    assert.equal(await page.getByRole('link', { name: 'Link to discussion' }).count(), 0)
    assert.equal(await page.getByRole('link', { name: 'Link to reply' }).count(), 0)
    assert.deepEqual(
      await threads.evaluateAll((elements) => elements.map(({ id }) => id)),
      [
        'discussion-discussion_demo_retry_cap',
        'discussion-discussion_demo_jitter',
        'discussion-discussion_demo_resolved_docs',
        'discussion-discussion_demo_revision_jitter',
        'discussion-discussion_demo_revision_tests',
        'discussion-discussion_demo_revision_final',
      ],
    )
    assert.equal(await page.getByRole('textbox').count(), 0)

    const resolvedThread = page.locator('#discussion-discussion_demo_resolved_docs')
    await resolvedThread.getByRole('button', { name: 'Show 1 reply' }).waitFor()
    await resolvedThread.getByText('The helper accepts milliseconds', {
      exact: false,
    }).waitFor()

    const retryThread = page.locator('#discussion-discussion_demo_retry_cap')
    await retryThread.getByRole('button', { name: 'Hide 3 replies' }).waitFor()
    const maintainerReply = page.locator(
      '#reply-discussion_reply_demo_retry_cap_maintainer',
    )
    await maintainerReply.getByText('Two seconds is intentional', { exact: false }).waitFor()
    const contributorReply = page.locator(
      '#reply-discussion_reply_demo_retry_cap_quote',
    )
    await contributorReply.getByText('Agreed. Quoting the maintainer', { exact: false }).waitFor()
    const nestedReply = page.locator('#reply-discussion_reply_demo_retry_cap_nested')
    await nestedReply.getByText('Exactly. Keeping that decision', { exact: false }).waitFor()
    assert.deepEqual(
      await retryThread.locator('[id^="reply-"]').evaluateAll((elements) =>
        elements.map(({ id }) => id),
      ),
      [
        'reply-discussion_reply_demo_retry_cap_maintainer',
        'reply-discussion_reply_demo_retry_cap_quote',
        'reply-discussion_reply_demo_retry_cap_nested',
      ],
    )
    await contributorReply
      .locator(
        'a[href="#discussion=discussion_demo_retry_cap&reply=discussion_reply_demo_retry_cap_maintainer"]',
      )
      .getByText('Two seconds is intentional', { exact: false })
      .waitFor()
    await nestedReply
      .locator(
        'a[href="#discussion=discussion_demo_retry_cap&reply=discussion_reply_demo_retry_cap_quote"]',
      )
      .getByText('Agreed. Quoting the maintainer', { exact: false })
      .waitFor()

    const hideRetryReplies = retryThread.getByRole('button', {
      name: 'Hide 3 replies',
    })
    const retryReplies = retryThread.locator(
      '#discussion-discussion_demo_retry_cap-replies',
    )
    await waitForClientHydration(page, hideRetryReplies)
    const disclosureTop = await hideRetryReplies.evaluate(
      (element) => element.getBoundingClientRect().top,
    )
    const scrollPosition = await page.evaluate(() => ({
      main: document.getElementById('main-content')?.scrollTop ?? null,
      window: window.scrollY,
    }))
    await hideRetryReplies.click()
    await assertReplyRegion(page, retryReplies, false)
    await retryThread
      .getByText('Should the retry cap remain', { exact: false })
      .waitFor()
    assert.equal(
      Math.round(
        await retryThread
          .getByRole('button', { name: 'Show 3 replies' })
          .evaluate((element) => element.getBoundingClientRect().top),
      ),
      Math.round(disclosureTop),
    )
    assert.deepEqual(
      await page.evaluate(() => ({
        main: document.getElementById('main-content')?.scrollTop ?? null,
        window: window.scrollY,
      })),
      scrollPosition,
    )
    await retryThread.getByRole('button', { name: 'Show 3 replies' }).click()
    await assertReplyRegion(page, retryReplies, true)
    await retryThread.getByRole('button', { name: 'Hide 3 replies' }).click()
    await retryThread.getByRole('button', { name: 'Show 3 replies' }).click()
    await assertReplyRegion(page, retryReplies, true)
    await retryThread.getByRole('button', { name: 'Hide 3 replies' }).click()
    await assertReplyRegion(page, retryReplies, false)

    const jitterThread = page.locator('#discussion-discussion_demo_jitter')
    const hideJitterReplies = jitterThread.getByRole('button', {
      name: 'Hide 1 reply',
    })
    const mainContent = page.locator('#main-content')
    await page.evaluate(() => {
      const hash = 'discussion=discussion_demo_jitter&reply=discussion_reply_demo_jitter'
      window.__scopeDiscussionHashRendered = false
      const unsubscribe = globalThis.__TSR_ROUTER__.subscribe('onRendered', (event) => {
        if (event.toLocation.hash !== hash) return
        window.__scopeDiscussionHashRendered = true
        unsubscribe()
      })
      window.location.hash = `#${hash}`
    })
    // The router marks its location resolved before onRendered restores scroll.
    // Wait for that event before measuring whether collapsing a reply moves it.
    await page.waitForFunction(() => window.__scopeDiscussionHashRendered &&
      document.activeElement?.id === 'reply-discussion_reply_demo_jitter')
    await hideJitterReplies.evaluate((element) => {
      element.scrollIntoView({ block: 'center' })
    })
    const deepScrollPosition = await mainContent.evaluate(
      (element) => element.scrollTop,
    )
    assert(deepScrollPosition > 0)
    await hideJitterReplies.click()
    await assertReplyRegion(
      page,
      jitterThread.locator('#discussion-discussion_demo_jitter-replies'),
      false,
    )
    assert.equal(
      await mainContent.evaluate((element) => element.scrollTop),
      deepScrollPosition,
    )

    const {
      heading: requestHeading,
      navigation: requestNavigation,
    } = await assertRequestCrossLinksStayInDocument(page)

    const requestViews = page.getByRole('navigation', { name: 'Request views' })
    const changesLink = requestViews.getByRole('link', { name: 'Changes' })
    await waitForClientHydration(page, changesLink)
    const transitionServerFunctions = []
    const recordServerFunction = (request) => {
      if (request.url().includes('/_serverFn/')) {
        transitionServerFunctions.push(new URL(request.url()).pathname)
      }
    }
    page.on('request', recordServerFunction)
    await changesLink.click()
    await page.waitForURL((url) => url.pathname.endsWith('/requests/req_demo_ready/changes'))
    await page.getByLabel('Commit file navigator').waitFor()
    page.off('request', recordServerFunction)
    const repeatedServerFunctions = transitionServerFunctions.filter(
      (url, index, requests) => requests.indexOf(url) !== index,
    )
    assert.deepEqual(repeatedServerFunctions, [])
    await assertRequestShellPreserved(page, {
      heading: requestHeading,
      navigation: requestNavigation,
    })
    await assertFileSelectionSkipsRevisionReload(page, 'retry.ts', '/src/retry.ts')
    await assertUpdateSelectionReloadsSelectedPayload(page)
    await page.getByRole('navigation', { name: 'Request views' })
      .getByRole('link', { name: 'Discussion' })
      .click()
    await page.waitForURL((url) => url.pathname.endsWith('/requests/req_demo_ready'))
    await page.locator('.request-discussion-thread').first().waitFor()
    const restoredRetryThread = page.locator('#discussion-discussion_demo_retry_cap')
    await restoredRetryThread
      .getByRole('button', { name: 'Show 3 replies' })
      .waitFor()
    const restoredRetryReplies = restoredRetryThread.locator(
      '#discussion-discussion_demo_retry_cap-replies',
    )
    await assertReplyRegion(page, restoredRetryReplies, false)
    await restoredRetryThread.getByRole('button', { name: 'Show 3 replies' }).click()
    await assertReplyRegion(page, restoredRetryReplies, true)
    await assertRequestShellPreserved(page, {
      heading: requestHeading,
      navigation: requestNavigation,
    })
  })
})

async function assertReplyRegion(page, region, expanded) {
  const id = await region.getAttribute('id')
  assert(id, 'reply region must have an id')
  await page.waitForFunction(
    ({ expectedExpanded, regionId }) => {
      const element = document.getElementById(regionId)
      if (!element) return false
      const height = element.getBoundingClientRect().height
      return expectedExpanded ? height > 0 : height < 1
    },
    { expectedExpanded: expanded, regionId: id },
  )
  assert.equal(await region.getAttribute('aria-hidden'), String(!expanded))
  assert.equal(await region.getAttribute('inert'), expanded ? null : '')
}

test('request details disclose on mobile without replacing discussion or quote targets', async () => {
  await withPage(`/${owner}/update-demo/requests/req_demo_ready`, async (page) => {
    await page.setViewportSize({ width: 390, height: 844 })
    const context = page.locator('.request-context-rail > details')
    const summary = context.locator(':scope > summary')
    await summary.waitFor()
    assert.equal(await context.getAttribute('open'), null)
    assert.doesNotMatch(await context.ariaSnapshot(), /Public request/)
    assert.equal(await context.count(), 1)
    const tabs = page.getByRole('navigation', { name: 'Request views' })
    const thread = page.locator('#discussion-discussion_demo_retry_cap')
    const tabBox = await tabs.boundingBox()
    const summaryBox = await summary.boundingBox()
    const threadBox = await thread.boundingBox()
    assert(tabBox.y + tabBox.height <= summaryBox.y)
    assert(summaryBox.y + summaryBox.height <= threadBox.y)
    await summary.click()
    await context.getByText('Public request', { exact: true }).waitFor()
    const invitees = context.locator('details').filter({ has: page.getByRole('heading', { name: 'invitees', exact: true }) })
    assert.equal(await invitees.getAttribute('open'), null)
    await invitees.locator('summary').click()
    await invitees.getByText('No invitees.', { exact: false }).waitFor()
    await summary.click()
    await page.setViewportSize({ width: 1440, height: 1000 })
    await context.getByText('Public request', { exact: true }).waitFor()
    assert.equal(await context.getAttribute('open'), '')
    assert.match(await context.ariaSnapshot(), /Public request/)
    assert.equal(await context.count(), 1)
    await page.setViewportSize({ width: 390, height: 844 })
    await page.waitForFunction(() => !document.querySelector('.request-context-rail > details').open)
    assert.doesNotMatch(await context.ariaSnapshot(), /Public request/)
    await page.setViewportSize({ width: 1440, height: 1000 })
    const quote = page.locator('#reply-discussion_reply_demo_retry_cap_quote a[href^="#discussion="]')
    await quote.click()
    await page.waitForFunction(() => document.activeElement?.id === 'reply-discussion_reply_demo_retry_cap_maintainer')
    assert.equal(await page.locator('h1').innerText(), 'Add bounded retry timing')
  })
})

test('details opened before hydration close on the next click and stay closed on mobile', async () => {
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 390, height: 844 } })
  let releaseScripts
  const scriptsHeld = new Promise((resolve) => { releaseScripts = resolve })
  try {
    await page.route('**/*', async (route) => {
      if (route.request().resourceType() === 'script') await scriptsHeld
      await route.continue().catch(() => {})
    })
    await page.goto(new URL(`/${owner}/update-demo/requests/req_demo_ready`, baseUrl).href, {
      waitUntil: 'commit',
    })
    const context = page.locator('.request-context-rail > details')
    const summary = context.locator(':scope > summary')
    await summary.waitFor()
    assert.equal(await summary.evaluate((element) => Object.keys(element).some((key) => key.startsWith('__reactProps$'))), false)
    await summary.click()
    assert.equal(await context.getAttribute('open'), '')
    releaseScripts()
    await waitForClientHydration(page, summary)
    await summary.click()
    await page.waitForFunction(() => !document.querySelector('.request-context-rail > details').open)
    await page.setViewportSize({ width: 1440, height: 1000 })
    await context.getByText('Public request', { exact: true }).waitFor()
    await page.setViewportSize({ width: 390, height: 844 })
    await page.waitForFunction(() => !document.querySelector('.request-context-rail > details').open)
  } finally {
    releaseScripts()
    await browser.close()
  }
})

test('mobile details close survives desktop resize before native toggle delivery', async () => {
  await withPage(`/${owner}/update-demo/requests/req_demo_ready`, async (page) => {
    await page.setViewportSize({ width: 390, height: 844 })
    const context = page.locator('.request-context-rail > details')
    const summary = context.locator(':scope > summary')
    await waitForClientHydration(page, summary)
    await page.waitForFunction(() => !document.querySelector('.request-context-rail > details').open)
    await summary.press('Enter')
    await context.getByText('Public request', { exact: true }).waitFor()
    const invitees = context.locator('details').filter({ has: page.getByRole('heading', { name: 'invitees', exact: true }) })
    await invitees.locator('summary').click()
    await invitees.getByText('No invitees.', { exact: false }).waitFor()
    const originalContext = await context.elementHandle()
    const originalInvitees = await invitees.elementHandle()

    await context.evaluate((element) => {
      window.__scopeHeldContextToggle = null
      const holdClose = (event) => {
        if (event.target !== element || event.newState !== 'closed') return
        event.stopImmediatePropagation()
        document.removeEventListener('toggle', holdClose, true)
        window.__scopeHeldContextToggle = { oldState: event.oldState, newState: event.newState }
      }
      document.addEventListener('toggle', holdClose, true)
    })
    await summary.press('Space')
    await page.waitForFunction(() => window.__scopeHeldContextToggle !== null)
    await page.setViewportSize({ width: 1440, height: 1000 })
    // Deliver the native close event after the desktop media-query render.
    // This reproduces the browser's deferred toggle ordering without replacing
    // the application component or reaching into its React state.
    await page.evaluate(() => new Promise((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(resolve))
    }))
    await originalContext.evaluate((element) => {
      element.dispatchEvent(new ToggleEvent('toggle', window.__scopeHeldContextToggle))
      delete window.__scopeHeldContextToggle
    })
    await context.getByText('Public request', { exact: true }).waitFor({ timeout: 5_000 })
    assert(await originalContext.evaluate((element) => element === document.querySelector('.request-context-rail > details')))
    assert(await originalInvitees.evaluate((element) => element.isConnected && element.open))

    await page.setViewportSize({ width: 390, height: 844 })
    await page.waitForFunction(() => !document.querySelector('.request-context-rail > details').open)
    await summary.press('Enter')
    await invitees.getByText('No invitees.', { exact: false }).waitFor()
    assert(await originalInvitees.evaluate((element) => element.isConnected && element.open))
  })
})

async function withPage(path, assertion) {
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage()
  const pageErrors = []
  page.on('pageerror', (error) => pageErrors.push(error.message))

  try {
    const response = await page.goto(new URL(path, `${baseUrl}/`).toString(), {
      timeout: 30_000,
      waitUntil: 'domcontentloaded',
    })
    assert(response, `navigation to ${path} did not produce a response`)
    assert(response.status() < 400, `navigation to ${path} returned ${response.status()}`)
    await assertion(page)
    assert.deepEqual(pageErrors, [])
  } finally {
    await browser.close()
  }
}
