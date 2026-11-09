import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  assertCurrentRepoSection,
  assertDocumentPreserved,
  assertNodesPreserved,
  assertPageHeading,
  assertPassiveSkeleton,
  captureRepositoryChrome,
  markDocument,
  repoPath,
  requestRepoPath,
  waitForClientHydration,
  within,
  withPage,
} from './browser-smoke.mjs'
import { serverFunctionName } from './server-functions-smoke.mjs'
import { assertUpdateFileSelectionKeepsDocument } from './update-navigation-smoke.mjs'
import { assertRepositoryMarkdownUsesClientNavigation } from './repository-markdown-navigation-smoke.mjs'

const primaryLink = (page, name) => page
  .getByRole('navigation', { name: 'Primary' })
  .getByRole('link', { name, exact: true })

test('central search finds repository history and opens its update page', async () => {
  await withPage(repoPath, async (page) => {
    await assertCurrentRepoSection(page, 'Home')
    const trigger = page.getByRole('combobox', { name: 'Search requests, history and files' })
    await waitForClientHydration(trigger)
    const documentSentinel = 'scope-history-menu-navigation'
    await markDocument(page, documentSentinel)
    const initialHistory = page.waitForResponse((response) =>
      serverFunctionName(response.request()) === 'loadHistoryPage_createServerFn_handler'
      && decodeURIComponent(response.url()).includes('Projected'))
    await trigger.fill('Projected')
    await initialHistory
    const update = page.getByRole('option', { name: /Projected public update/ })
    await update.waitFor()
    let releaseHistory
    let historyStarted
    const heldHistory = new Promise((resolve) => { releaseHistory = resolve })
    const filteredHistory = new Promise((resolve) => { historyStarted = resolve })
    await page.route('**/_serverFn/**', async (route) => {
      if (serverFunctionName(route.request()) === 'loadHistoryPage_createServerFn_handler') {
        historyStarted()
        await heldHistory
      }
      await route.continue()
    })
    await page.clock.install()
    await page.clock.pauseAt(new Date())
    try {
      await trigger.fill('Projected public')
      assert.equal(await update.count(), 0, 'Previous-query options must not be offered during debounce')
      await page.clock.runFor(200)
      await within(filteredHistory, 10_000, 'filtered history search did not start')
      assert.equal(await update.count(), 0, 'Previous-query options must not be offered while search is pending')
    } finally {
      releaseHistory()
      await page.clock.resume()
    }
    await update.waitFor()
    await update.click()
    await page.waitForURL((url) => url.pathname === `${repoPath}/code` && url.searchParams.get('update') === 'dev-public-1')
    await page.getByRole('link', { name: 'View changes', exact: true }).click()
    await page.waitForURL((url) => url.pathname === `${repoPath}/updates/dev-public-1`)
    await assertCurrentRepoSection(page, 'Home')
    await page.getByRole('heading', { name: 'Projected public update', exact: true }).waitFor()
    await assertDocumentPreserved(page, documentSentinel)
    await assertUpdateFileSelectionKeepsDocument(page)
    assert.equal(
      await page.evaluate(() => document.querySelector('#main-content')?.scrollTop),
      0,
    )
  })
})

test('repository Markdown routes relative file links without replacing the document', async () => {
  await withPage(`${requestRepoPath}/code`, assertRepositoryMarkdownUsesClientNavigation)
})

test('repository chrome persists across navigation and request revalidation', async () => {
  await withPage(requestRepoPath, async (page) => {
    await assertCurrentRepoSection(page, 'Home')
    await waitForClientHydration(primaryLink(page, 'Requests'))
    const chrome = await captureRepositoryChrome(page)

    const primaryNavigation = page.getByRole('navigation', { name: 'Primary' })
    await primaryNavigation
      .getByRole('link', { name: 'Requests', exact: true })
      .click()
    await assertCurrentRepoSection(page, 'Requests')
    await assertNodesPreserved(page, chrome)
    await assertCurrentRepoSection(page, 'Requests')

    await page
      .getByRole('link', { name: /Add bounded retry timing/ })
      .click()
    await page
      .getByRole('heading', { level: 1, name: 'Add bounded retry timing' })
      .waitFor()
    await assertNodesPreserved(page, chrome)
    await assertCurrentRepoSection(page, 'Requests')

    await page.evaluate(() => globalThis.__TSR_ROUTER__.invalidate())
    await assertNodesPreserved(page, chrome)
    await assertCurrentRepoSection(page, 'Requests')

    await page.locator('#main-content').evaluate(async (element) => {
      await new Promise((resolve) => {
        element.addEventListener('scroll', resolve, { once: true })
        element.scrollTop = 500
      })
    })
    await primaryNavigation
      .getByRole('link', { name: 'Requests', exact: true })
      .click()
    await assertCurrentRepoSection(page, 'Requests')
    await page.waitForFunction(
      () => document.querySelector('#main-content')?.scrollTop === 0,
    )
    await assertNodesPreserved(page, chrome)

    await page.goBack()
    await page
      .getByRole('heading', { level: 1, name: 'Add bounded retry timing' })
      .waitFor()
    await assertNodesPreserved(page, chrome)
    await assertCurrentRepoSection(page, 'Requests')

    await primaryNavigation
      .getByRole('link', { name: 'Requests', exact: true })
      .click()
    await assertCurrentRepoSection(page, 'Requests')

    await armNextRouterLoad(page)
    await primaryNavigation
      .getByRole('link', { name: 'Requests', exact: true })
      .click()
    await page.waitForFunction(() => globalThis.__scopeRouterLoaded === true)
    await assertNodesPreserved(page, chrome)
    await assertCurrentRepoSection(page, 'Requests')

    await primaryNavigation
      .getByRole('link', { name: 'Home', exact: true })
      .click()
    await assertCurrentRepoSection(page, 'Home')
    await assertNodesPreserved(page, chrome)
    await assertCurrentRepoSection(page, 'Home')
  })
})

test('requests navigation shows a destination skeleton inside the repository shell', async () => {
  await withPage(`${repoPath}/code`, async (page) => {
    await page.emulateMedia({ reducedMotion: 'reduce' })
    await assertCurrentRepoSection(page, 'Home')
    await assertPageHeading(page, 'Code')
    await page.getByLabel('Repository file navigator').waitFor()
    await waitForClientHydration(primaryLink(page, 'Requests'))
    const chrome = await captureRepositoryChrome(page)

    let releaseQueueRequests = () => undefined
    let markQueueRequestStarted = () => undefined
    const queueRequestStarted = new Promise((resolve) => {
      markQueueRequestStarted = resolve
    })
    const queueRequestsReleased = new Promise((resolve) => {
      releaseQueueRequests = resolve
    })
    await page.route('**/_serverFn/**', async (route) => {
      const requestUrl = decodeURIComponent(route.request().url())
      if (!requestUrl.includes('section')) {
        await route.continue()
        return
      }
      markQueueRequestStarted()
      await queueRequestsReleased
      await route.continue()
    })

    const requestsNavigation = primaryLink(page, 'Requests').click()

    try {
      await within(
        queueRequestStarted,
        10_000,
        'request queue navigation did not start',
      )
      const pendingPage = page.locator('#main-content [aria-busy="true"]').first()
      await pendingPage.waitFor()
      await assertCurrentRepoSection(page, 'Requests')
      await assertNodesPreserved(page, chrome)
      await page
        .getByLabel('Repository file navigator')
        .waitFor({ state: 'detached' })
      await page
        .getByRole('heading', { level: 1, name: 'Code' })
        .waitFor({ state: 'detached' })
      await assertPassiveSkeleton(page, '#main-content')
      const reducedMotion = await page
        .locator('#main-content [data-slot="skeleton"]')
        .first()
        .evaluate((element) => {
          const style = getComputedStyle(element)
          return {
            durationSeconds: Number.parseFloat(style.animationDuration),
            iterations: style.animationIterationCount,
          }
        })
      assert.equal(reducedMotion.durationSeconds <= 0.001, true)
      assert.equal(reducedMotion.iterations, '1')
    } finally {
      releaseQueueRequests()
      await requestsNavigation
    }

    await page.getByRole('complementary', { name: 'Requests workspace' }).waitFor()
    await page.locator('#main-content [data-slot="skeleton"]').first().waitFor({
      state: 'detached',
    })
    assert.equal(
      await page.locator('#main-content [data-slot="skeleton"]').count(),
      0,
    )
  })
})

async function armNextRouterLoad(page) {
  await page.evaluate(() => {
    globalThis.__scopeRouterLoaded = false
    const unsubscribe = globalThis.__TSR_ROUTER__.subscribe(
      'onLoad',
      () => {
        unsubscribe()
        globalThis.__scopeRouterLoaded = true
      },
    )
  })
}
