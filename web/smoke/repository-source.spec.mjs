import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  assertCurrentRepoSection,
  assertPageHeading,
  assertPassiveSkeleton,
  repoPath,
  waitForClientHydration,
  within,
  withPage,
} from './browser-smoke.mjs'

test('public repository exposes only its projected source', async () => {
  await withPage(repoPath, async (page) => {
    await assertCurrentRepoSection(page, 'Code')
    await assertPageHeading(page, 'Code')
    await page.getByText('2 files', { exact: true }).waitFor()
    await page.getByRole('tab', { name: 'README.html' }).waitFor()
    await page.getByRole('button', { name: 'README.html', exact: true }).waitFor()
    const previewButton = page.getByRole('radio', { name: 'Preview', exact: true })
    const sourceButton = page.getByRole('radio', { name: 'Source', exact: true })
    await previewButton.waitFor()
    await sourceButton.waitFor()

    const preview = page.locator('iframe[title="README.html preview"]')
    await preview.waitFor()
    const sandbox = await preview.getAttribute('sandbox')
    assert.notEqual(sandbox, null)
    const sandboxCapabilities = sandbox.split(/\s+/)
    for (const capability of [
      'allow-forms',
      'allow-popups',
      'allow-popups-to-escape-sandbox',
      'allow-same-origin',
      'allow-scripts',
      'allow-top-navigation',
    ]) {
      assert.equal(sandboxCapabilities.includes(capability), false)
    }

    const previewDocument = page.frameLocator('iframe[title="README.html preview"]')
    await previewDocument
      .getByRole('heading', { level: 1, name: 'Public by design.' })
      .waitFor()
    const contentSecurityPolicy = await previewDocument
      .locator('meta[http-equiv="Content-Security-Policy"]')
      .getAttribute('content')
    assert.match(contentSecurityPolicy, /script-src 'none'/)
    assert.match(contentSecurityPolicy, /connect-src 'none'/)

    const previewElement = await preview.elementHandle()
    assert(previewElement)
    const previewFrame = await previewElement.contentFrame()
    assert(previewFrame)

    let networkRequests = 0
    const networkProbeUrl = 'https://example.com/README-sandbox-check'
    await page.route(networkProbeUrl, async (route) => {
      networkRequests += 1
      await route.fulfill({ status: 204 })
    })
    assert.equal(
      await previewFrame.evaluate(async (url) => {
        try {
          await fetch(url, { mode: 'no-cors' })
          return 'allowed'
        } catch {
          return 'blocked'
        }
      }, networkProbeUrl),
      'blocked',
    )
    assert.equal(networkRequests, 0)
    await previewFrame.evaluate(() => {
      const script = document.createElement('script')
      script.textContent = 'document.documentElement.dataset.scriptRan = "true"'
      document.head.append(script)
    })
    assert.equal(
      await previewFrame.evaluate(() => document.documentElement.dataset.scriptRan),
      undefined,
    )
    assert.equal(
      await previewFrame.evaluate(() => {
        try {
          return parent.document.body !== null
        } catch {
          return false
        }
      }),
      false,
    )

    await previewFrame.evaluate(() => {
      document.documentElement.dataset.persistenceProbe = 'same-document'
    })

    await waitForClientHydration(page.getByRole('tab', { name: 'README.html', exact: true }))
    const readmeHistoryLength = await page.evaluate(() => history.length)
    const selectedReadmeRequests = []
    const recordSelectedReadmeRequest = (request) => {
      if (request.url().includes('/_serverFn/')) {
        selectedReadmeRequests.push(request.url())
      }
    }
    page.on('request', recordSelectedReadmeRequest)
    await page.getByRole('button', { name: 'README.html', exact: true }).click()
    await page.getByRole('tab', { name: 'README.html', exact: true }).dblclick()
    page.off('request', recordSelectedReadmeRequest)
    assert.deepEqual(selectedReadmeRequests, [])
    assert.equal(new URL(page.url()).searchParams.has('file'), false)
    assert.equal(await page.evaluate(() => history.length), readmeHistoryLength)
    await preview.waitFor()
    assert.equal(
      await previewFrame.evaluate(() =>
        document.documentElement.dataset.persistenceProbe
      ),
      'same-document',
    )

    await sourceButton.click()
    await page.locator('pre code').filter({ hasText: '<!doctype html>' }).waitFor()
    assert.equal(await preview.isVisible(), false)
    await previewButton.click()
    await previewDocument
      .getByRole('heading', { level: 1, name: 'Public by design.' })
      .waitFor()
    assert.equal(
      await previewFrame.evaluate(() =>
        document.documentElement.dataset.persistenceProbe
      ),
      'same-document',
    )

    await page
      .getByRole('navigation', { name: 'Primary' })
      .getByRole('link', { name: 'Requests', exact: true })
      .click()
    await assertPageHeading(page, 'Requests')
    const codeReturnRequests = []
    const recordCodeReturnRequest = (request) => {
      if (request.url().includes('/_serverFn/')) {
        codeReturnRequests.push(decodeURIComponent(request.url()))
      }
    }
    page.on('request', recordCodeReturnRequest)
    await page.evaluate(() => {
      globalThis.__scopeCodeReturnSawLoading = false
      globalThis.__scopeTrackCodeReturn = true
      const sample = () => {
        if (!globalThis.__scopeTrackCodeReturn) return
        if (document.body.textContent?.includes('Loading repository files')) {
          globalThis.__scopeCodeReturnSawLoading = true
        }
        requestAnimationFrame(sample)
      }
      requestAnimationFrame(sample)
    })
    await page
      .getByRole('navigation', { name: 'Primary' })
      .getByRole('link', { name: 'Code', exact: true })
      .click()
    await assertPageHeading(page, 'Code')
    await preview.waitFor()
    const sawCodeReturnLoading = await page.evaluate(() => {
      globalThis.__scopeTrackCodeReturn = false
      return globalThis.__scopeCodeReturnSawLoading
    })
    page.off('request', recordCodeReturnRequest)
    assert.equal(sawCodeReturnLoading, false)
    assert.equal(
      codeReturnRequests.some((request) => request.includes('loadRepoContent')),
      false,
    )
    assert.equal(
      await previewFrame.evaluate(() =>
        document.documentElement.dataset.persistenceProbe
      ),
      'same-document',
    )
    const navigator = page.getByLabel('Repository file navigator')
    await navigator.evaluate((element) => {
      element.dataset.persistenceProbe = 'same-route'
    })
    let appFileRequests = 0
    let releaseFileRequest = () => undefined
    let markFileRequestStarted = () => undefined
    const fileRequestStarted = new Promise((resolve) => {
      markFileRequestStarted = resolve
    })
    const fileRequestReleased = new Promise((resolve) => {
      releaseFileRequest = resolve
    })
    await page.route('**/_serverFn/**', async (route) => {
      if (
        route.request().method() !== 'GET' ||
        !decodeURIComponent(route.request().url()).includes('src/app.ts')
      ) {
        await route.continue()
        return
      }
      appFileRequests += 1
      markFileRequestStarted()
      await fileRequestReleased
      await route.continue()
    })
    await page.evaluate(() => {
      globalThis.__scopeTransitionFrames = {
        emptyViewer: false,
        hiddenRepository: false,
      }
      globalThis.__scopeTrackTransitionFrames = true
      const sample = () => {
        if (!globalThis.__scopeTrackTransitionFrames) return
        if (document.body.textContent?.includes(
          'Select a file to inspect its contents.',
        )) {
          globalThis.__scopeTransitionFrames.emptyViewer = true
        }
        const navigator = document.querySelector(
          '[aria-label="Repository file navigator"]',
        )
        if (!navigator || navigator.getClientRects().length === 0) {
          globalThis.__scopeTransitionFrames.hiddenRepository = true
        }
        requestAnimationFrame(sample)
      }
      requestAnimationFrame(sample)
    })
    const expandSrc = page.getByRole('button', { name: 'Expand src' })
    await waitForClientHydration(expandSrc)
    await expandSrc.click()
    const openFile = page.getByRole('button', { name: 'app.ts', exact: true }).click()
    await within(fileRequestStarted, 10_000, 'file request did not start')
    await page.waitForURL((url) => url.searchParams.get('file') === 'src/app.ts')
    const pendingFileViewer = page.locator(
      '#repository-code-files-panel [aria-busy="true"]',
    )
    await pendingFileViewer.waitFor()
    await assertPassiveSkeleton(page, '#repository-code-files-panel')
    assert.equal(await preview.isVisible(), false)
    assert.equal(
      await navigator.getAttribute('data-persistence-probe'),
      'same-route',
    )
    releaseFileRequest()
    await openFile
    await page.locator('pre code').filter({ hasText: 'export function greet' }).waitFor()
    assert.equal(
      await page.locator('#repository-code-files-panel [data-slot="skeleton"]').count(),
      0,
    )
    assert.equal(appFileRequests, 1)
    const transitionFrames = await page.evaluate(() => {
      globalThis.__scopeTrackTransitionFrames = false
      return globalThis.__scopeTransitionFrames
    })
    assert.equal(transitionFrames.emptyViewer, false)
    assert.equal(transitionFrames.hiddenRepository, false)
    assert.equal(
      await navigator.getAttribute('data-persistence-probe'),
      'same-route',
    )
    await page.goBack()
    await page.waitForURL((url) => !url.searchParams.has('file'))
    await preview.waitFor()
    assert.equal(
      await navigator.getAttribute('data-persistence-probe'),
      'same-route',
    )
    assert.equal(
      await previewFrame.evaluate(() =>
        document.documentElement.dataset.persistenceProbe
      ),
      'same-document',
    )
    await page.getByRole('button', { name: 'app.ts', exact: true }).click()
    await page.locator('pre code').filter({ hasText: 'export function greet' }).waitFor()
    assert.equal(appFileRequests, 1)
    await page.goBack()
    await page.waitForURL((url) => !url.searchParams.has('file'))
    await preview.waitFor()
    await page.reload()
    await preview.waitFor()
    const failedNavigator = page.getByLabel('Repository file navigator')
    const expandFailedSrc = page.getByRole('button', { name: 'Expand src' })
    await waitForClientHydration(expandFailedSrc)
    await failedNavigator.evaluate((element) => {
      element.dataset.persistenceProbe = 'failed-file-local'
    })
    await page.unroute('**/_serverFn/**')
    await page.route('**/_serverFn/**', async (route) => {
      if (
        route.request().method() !== 'GET' ||
        !decodeURIComponent(route.request().url()).includes('src/app.ts')
      ) {
        await route.continue()
        return
      }
      await route.fulfill({ body: 'file unavailable', status: 500 })
    })
    await expandFailedSrc.click()
    await page.getByRole('button', { name: 'app.ts', exact: true }).click()
    await page.waitForURL((url) => url.searchParams.get('file') === 'src/app.ts')
    await page.getByRole('button', { name: 'Retry', exact: true }).waitFor()
    assert.equal(
      await failedNavigator.getAttribute('data-persistence-probe'),
      'failed-file-local',
    )
    assert.equal(await page.getByText('internal', { exact: true }).count(), 0)
    assert.equal(await page.getByText('plan.md', { exact: true }).count(), 0)
    assert.equal(
      await page
        .getByRole('navigation', { name: 'Primary' })
        .getByRole('link', { name: 'Runs', exact: true })
        .count(),
      0,
    )
    assert.equal(
      await page.getByRole('heading', { name: 'Recent runs' }).count(),
      0,
    )
    assert.equal(await page.getByRole('heading', { name: 'Runners' }).count(), 0)
  })
})
