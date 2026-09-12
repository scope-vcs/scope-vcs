import assert from 'node:assert/strict'
import { test } from 'node:test'
import { chromium } from 'playwright'
import { serverFunctionName } from './server-functions-smoke.mjs'
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

test('discussion and reply chronology preserves quote targets', async () => {
  await withPage(`/${owner}/update-demo/requests/req_demo_ready`, async (page) => {
    const retryThread = page.locator('#discussion-discussion_demo_retry_cap')
    const resolvedThread = page.locator('#discussion-discussion_demo_resolved_docs')
    await resolvedThread.getByRole('button', { name: 'Show 1 reply' }).waitFor()
    await resolvedThread.getByText('The helper accepts milliseconds', { exact: false }).waitFor()
    await waitForClientHydration(page, retryThread.getByRole('button', { name: 'Hide 3 replies' }))
    assert.deepEqual(
      await page.locator('.request-discussion-thread').evaluateAll((elements) =>
        elements.map(({ id }) => id),
      ),
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

    const maintainerReply = retryThread.locator('#reply-discussion_reply_demo_retry_cap_maintainer')
    const contributorReply = retryThread.locator('#reply-discussion_reply_demo_retry_cap_quote')
    const nestedReply = retryThread.locator('#reply-discussion_reply_demo_retry_cap_nested')
    await nestedReply.waitFor()
    await assertBefore(maintainerReply, contributorReply)
    await assertBefore(contributorReply, nestedReply)
    for (const [reply, target] of [[contributorReply, maintainerReply], [nestedReply, contributorReply]]) {
      const quote = reply.locator('a[href^="#discussion="]')
      await quote.click()
      await page.waitForFunction((id) => document.activeElement?.id === id, await target.getAttribute('id'))
    }
  })
})

test('reply disclosure preserves scroll and remains reversible', async () => {
  await withPage(`/${owner}/update-demo/requests/req_demo_ready`, async (page) => {
    await page.locator('.request-workspace-sidebar a[href$="/req_demo_ready"]').waitFor()
    const retryThread = page.locator('#discussion-discussion_demo_retry_cap')
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
  })
})

test('revision and discussion links retain the document and request shell', async () => {
  await withPage(`/${owner}/update-demo/requests/req_demo_ready`, assertRequestCrossLinksStayInDocument)
})

test('changes navigation preserves the request shell and collapsed replies', async () => {
  await withPage(`/${owner}/update-demo/requests/req_demo_ready`, async (page) => {
    const requestHeading = await page.getByRole('heading', { level: 1 }).elementHandle()
    const requestNavigation = await page.getByRole('navigation', { name: 'Request views' }).elementHandle()
    assert(requestHeading)
    assert(requestNavigation)
    const retryThread = page.locator('#discussion-discussion_demo_retry_cap')
    const disclosure = retryThread.getByRole('button', { name: 'Hide 3 replies' })
    await waitForClientHydration(page, disclosure)
    await disclosure.click()
    await assertReplyRegion(page, retryThread.locator('#discussion-discussion_demo_retry_cap-replies'), false)
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

test('file and update selection reload only the selected changes payload', async () => {
  await withPage(`/${owner}/update-demo/requests/req_demo_ready/changes`, async (page) => {
    await page.getByLabel('Commit file navigator').waitFor()
    await assertFileSelectionSkipsRevisionReload(page, 'retry.ts', '/src/retry.ts')
    await assertUpdateSelectionReloadsSelectedPayload(page)
  })
})

async function assertBefore(first, second) {
  const next = await second.elementHandle()
  assert(next)
  assert(await first.evaluate((element, next) => Boolean(element.compareDocumentPosition(next) & Node.DOCUMENT_POSITION_FOLLOWING), next))
}

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

test('Details is a separate tab that reuses request data and preserves discussion state', async () => {
  await withPage(`/${owner}/update-demo/requests/req_demo_ready`, async (page) => {
    const tabs = page.getByRole('navigation', { name: 'Request views' })
    const details = tabs.getByRole('link', { name: 'Details', exact: true })
    const thread = page.locator('#discussion-discussion_demo_retry_cap')
    const collapse = thread.getByRole('button', { name: 'Hide 3 replies' })
    await waitForClientHydration(page, collapse)
    await collapse.click()
    await thread.getByRole('button', { name: 'Show 3 replies' }).waitFor()
    const shell = {
      heading: await page.getByRole('heading', { name: 'Add bounded retry timing', exact: true }).elementHandle(),
      navigation: await tabs.elementHandle(),
    }
    const header = page.locator('header').filter({ has: page.getByRole('heading', { name: 'Add bounded retry timing', exact: true }) })
    assert.equal(await header.getByText('Open', { exact: true }).getAttribute('data-variant'), 'success')
    assert.equal(await header.getByText('Open request', { exact: true }).count(), 0)
    assert.equal(await page.getByRole('navigation', { name: 'Request navigation' }).count(), 0)
    const requestLoads = []
    page.on('request', (request) => {
      if (serverFunctionName(request) === 'loadRequestPage_createServerFn_handler') requestLoads.push(request.url())
    })
    await details.click()
    await page.waitForURL((url) => url.pathname.endsWith('/req_demo_ready/details'))
    const context = page.getByRole('region', { name: 'Request details' })
    await context.getByText('Public request', { exact: true }).waitFor()
    assert.equal(await details.getAttribute('aria-current'), 'page')
    assert.equal(await page.locator('.request-discussion-thread').count(), 0)
    await assertRequestShellPreserved(page, shell)
    await page.setViewportSize({ width: 390, height: 844 })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth), false)
    await tabs.getByRole('link', { name: 'Discussion', exact: true }).click()
    await page.waitForURL((url) => url.pathname.endsWith('/req_demo_ready'))
    await thread.getByRole('button', { name: 'Show 3 replies' }).waitFor()
    await assertRequestShellPreserved(page, shell)
    assert.deepEqual(requestLoads, [])
    await thread.getByRole('button', { name: 'Show 3 replies' }).click()
    const quote = page.locator('#reply-discussion_reply_demo_retry_cap_quote a[href^="#discussion="]')
    await quote.click()
    await page.waitForFunction(() => document.activeElement?.id === 'reply-discussion_reply_demo_retry_cap_maintainer')
  })
})

test('Details tab has a working link before hydration and renders directly on mobile', async () => {
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ javaScriptEnabled: false, viewport: { width: 390, height: 844 } })
  try {
    await page.goto(new URL(`/${owner}/update-demo/requests/req_demo_ready`, baseUrl).href, { waitUntil: 'domcontentloaded' })
    await page.getByRole('navigation', { name: 'Request views' }).getByRole('link', { name: 'Details', exact: true }).click()
    await page.waitForURL((url) => url.pathname.endsWith('/req_demo_ready/details'))
    await page.getByRole('region', { name: 'Request details' }).getByText('Public request', { exact: true }).waitFor()
    assert.equal(await page.getByRole('heading', { name: 'Add bounded retry timing', exact: true }).count(), 1)
  } finally {
    await browser.close()
  }
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
