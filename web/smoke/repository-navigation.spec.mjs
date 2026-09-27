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
import { assertUpdateFileSelectionKeepsDocument } from './update-navigation-smoke.mjs'
import { assertRepositoryMarkdownUsesClientNavigation } from './repository-markdown-navigation-smoke.mjs'
import { trackRepositoryRefresh } from './repo-refresh-smoke.mjs'

const primaryLink = (page, name) => page
  .getByRole('navigation', { name: 'Primary' })
  .getByRole('link', { name, exact: true })

test('public repository history opens from the latest change and lists its seeded push', async () => {
  let settled
  await withPage(repoPath, async (page) => {
    await assertCurrentRepoSection(page, 'Code')
    const trigger = page
      .getByLabel('Latest repository change')
      .getByRole('button', { name: 'History', exact: true })
    await waitForClientHydration(trigger)
    await settled()
    const documentSentinel = 'scope-history-menu-navigation'
    await markDocument(page, documentSentinel)
    await trigger.click()
    const menu = page.getByRole('dialog', { name: 'Repository history' })
    const update = menu.getByRole('link', { name: /Projected public update/ })
    await update.waitFor()
    await update.getByText('2 files', { exact: true }).waitFor()
    await update.click()
    await page.waitForURL((url) => url.pathname === `${repoPath}/updates/dev-public-1`)
    await assertCurrentRepoSection(page, 'Code')
    await page.getByRole('heading', { name: 'Projected public update', exact: true }).waitFor()
    await assertDocumentPreserved(page, documentSentinel)
    await assertUpdateFileSelectionKeepsDocument(page)
    assert.equal(
      await page.evaluate(() => document.querySelector('#main-content')?.scrollTop),
      0,
    )
  }, { prepare: page => { settled = trackRepositoryRefresh(page) } })
})

test('repository Markdown routes relative file links without replacing the document', async () => {
  await withPage(requestRepoPath, assertRepositoryMarkdownUsesClientNavigation)
})

test('repository sections no longer include a History tab', async () => {
  await withPage(repoPath, async (page) => {
    await assertCurrentRepoSection(page, 'Code')
    await primaryLink(page, 'Requests').waitFor()
    assert.equal(await primaryLink(page, 'History').count(), 0)
  })
})

test('repository chrome persists across navigation and request revalidation', async () => {
  await withPage(requestRepoPath, async (page) => {
    await assertCurrentRepoSection(page, 'Code')
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
      .getByRole('link', { name: 'Code', exact: true })
      .click()
    await assertCurrentRepoSection(page, 'Code')
    await assertNodesPreserved(page, chrome)
    await assertCurrentRepoSection(page, 'Code')
  })
})

test('requests navigation shows a destination skeleton inside the repository shell', async () => {
  await withPage(repoPath, async (page) => {
    await page.emulateMedia({ reducedMotion: 'reduce' })
    await assertCurrentRepoSection(page, 'Code')
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
