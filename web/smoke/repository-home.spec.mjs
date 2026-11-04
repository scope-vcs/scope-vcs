import assert from 'node:assert/strict'
import { test } from 'node:test'
import { assertMobileFilesCollapsed, baseUrl, repoPath, requestRepoPath, waitForClientHydration, withPage } from './browser-smoke.mjs'
import { serverFunctionName } from './server-functions-smoke.mjs'

const viewport = { width: 1280, height: 900 }

test('Home shows only the README and Browse code opens the file tree', async () => {
  await withPage(requestRepoPath, async (page) => {
    const readme = page.getByRole('region', { name: 'Repository README' })
    await readme.locator('article h1').waitFor()
    assert.equal(await page.getByLabel('Repository file navigator').count(), 0)
    assert.equal(await page.getByLabel('Latest repository change', { exact: true }).count(), 0)
    assert.equal(await page.getByRole('alert').count(), 0)
    const browse = page.getByRole('link', { name: 'Browse code', exact: true })
    await waitForClientHydration(browse)
    await browse.click()
    await page.waitForURL((url) => url.pathname === `${requestRepoPath}/code`)
    await page.getByRole('tab', { name: 'README.md', exact: true }).waitFor()
    await page.getByLabel('Repository file navigator').waitFor()
    await page.goto(`${baseUrl}${repoPath}/code?file=src%2Fapp.ts`)
    await page.getByRole('tab', { name: 'src/app.ts', exact: true }).waitFor()
    await page.locator('pre code').filter({ hasText: 'export function greet' }).waitFor()
  }, { viewport })
})

test('central search opens files, keeps the open file, and honors keyboard rules', async () => {
  await withPage(`${repoPath}/code`, async (page) => {
    await page.getByRole('tab', { name: 'README.html', exact: true }).waitFor()
    await page.getByRole('tab', { name: 'README.html', exact: true }).dblclick()
    const search = page.getByRole('combobox', { name: 'Search requests, history and files' })
    await waitForClientHydration(search)
    await page.getByRole('tab', { name: 'README.html', exact: true }).focus()
    await page.keyboard.press('/')
    await page.waitForFunction(() => document.activeElement?.getAttribute('role') === 'combobox')
    await page.getByRole('radio', { name: 'Files', exact: true }).click()
    await search.fill('src/')
    await page.getByRole('option', { name: 'src/app.ts' }).waitFor()
    await search.focus()
    await page.keyboard.press('Enter')
    await page.locator('pre code').filter({ hasText: 'export function greet' }).waitFor()
    assert.equal(new URL(page.url()).searchParams.get('file'), 'src/app.ts')
    assert.equal(await page.getByRole('tab', { name: 'README.html', exact: true }).count(), 1)
    await search.fill('does-not-exist')
    await page.getByText('No matches.', { exact: true }).waitFor()
    await page.keyboard.press('Escape')
    assert.equal(await search.inputValue(), '')
    assert.equal(await page.getByRole('listbox', { name: 'Search results' }).count(), 0)
    await page.evaluate(() => {
      const textarea = document.createElement('textarea')
      textarea.dataset.finderTest = 'true'
      document.body.append(textarea)
      textarea.focus()
    })
    await page.keyboard.press('/')
    assert.equal(await page.locator('textarea[data-finder-test]').inputValue(), '/')
    await page.locator('textarea[data-finder-test]').evaluate((element) => element.remove())
    await page.setViewportSize({ width: 390, height: 844 })
    await page.getByRole('tab', { name: 'src/app.ts', exact: true }).focus()
    await page.keyboard.press('/')
    await page.waitForFunction(() => document.activeElement?.getAttribute('role') === 'combobox')
    await search.fill('README')
    await page.getByRole('option', { name: 'README.html' }).waitFor()
    await search.focus()
    await page.keyboard.press('Enter')
    await page.getByRole('tab', { name: 'README.html', exact: true }).waitFor()
    await assertMobileFilesCollapsed(page, 'README.html')
  }, { viewport })
})

test('README details preserve inspection without exposing repeated navigator metadata', async () => {
  await withPage(`${repoPath}/code`, async (page) => {
    const preview = page.locator('iframe[title="README.html preview"]')
    await preview.waitFor()
    assert.equal(await preview.getAttribute('sandbox'), '')
    assert.equal(await page.getByLabel('Repository file navigator').getByText('Tracked', { exact: true }).count(), 0)
    assert.equal(await page.getByText('Sandboxed document', { exact: true }).isVisible(), false)
    const details = page.getByRole('button', { name: 'File details', exact: true })
    const detailsPanel = page.getByRole('dialog', { name: 'File details', exact: true })
    await waitForClientHydration(details)
    await details.click()
    await detailsPanel.getByText(/^Blob:/).waitFor()
    await detailsPanel.getByText('Sandboxed document. Repository HTML runs in an isolated preview.', { exact: true }).waitFor()
    await page.keyboard.press('Escape')
    assert.equal(await detailsPanel.isVisible(), false)

    await page.goto(`${baseUrl}${repoPath}/code?file=src%2Fapp.ts`)
    await page.locator('pre code').waitFor()
    assert.equal(await page.getByText(/^\d+ B · /).count(), 0)
    await waitForClientHydration(details)
    await details.focus()
    await details.press('Enter')
    await detailsPanel.getByText(/^Blob: [0-9a-f]{40}$/).waitFor()
    await page.keyboard.press('Escape')
    assert.equal(await detailsPanel.isVisible(), false)
    await page.setViewportSize({ width: 390, height: 844 })
    await details.click()
    await detailsPanel.getByText(/^Blob: [0-9a-f]{40}$/).waitFor()
    const bounds = await detailsPanel.boundingBox()
    assert(bounds && bounds.x >= 0 && bounds.x + bounds.width <= 390)
  }, { viewport })
})

async function navigateFromHome(page, path) {
  await waitForClientHydration(page.getByRole('button', { name: 'System', exact: true }))
  const url = new URL(path, baseUrl)
  await page.evaluate(({ to, search }) => { void globalThis.__TSR_ROUTER__.navigate({ to, search }) }, {
    to: url.pathname,
    search: Object.fromEntries(url.searchParams),
  })
}

test('an explicit file remains readable while its tree is pending and after the tree fails', async () => {
  let releaseTree
  const held = new Promise((resolve) => { releaseTree = resolve })
  let intercepted = false
  const holdTree = (page) => page.route('**/_serverFn/**', async (route) => {
    if (serverFunctionName(route.request()) === 'loadRepoContent_createServerFn_handler') {
      intercepted = true
      await held
      await route.abort('failed').catch(() => {})
    } else await route.continue()
  })
  await withPage('/', async (page) => {
    try {
      await navigateFromHome(page, `${repoPath}/code?file=src%2Fapp.ts`)
      await page.locator('pre code').filter({ hasText: 'export function greet' }).waitFor()
      assert.equal(intercepted, true)
      releaseTree()
      await page.getByLabel('Repository file navigator').getByRole('alert').waitFor()
      assert.equal(await page.locator('pre code').filter({ hasText: 'export function greet' }).isVisible(), true)
      assert.equal(new URL(page.url()).searchParams.get('file'), 'src/app.ts')
    } finally { releaseTree() }
  }, { prepare: holdTree, viewport })
})

test('views without a README or any files open deliberately without guessing a file request', async () => {
  for (const files of [
    [{ path: '/src/app.ts', oid: 'test-oid', tracked: true, label: 'public' }],
    [],
  ]) {
    let fileRequests = 0
    const fulfillTree = (page) => page.route('**/_serverFn/**', async (route) => {
      const name = serverFunctionName(route.request())
      if (name === 'loadRepoContent_createServerFn_handler') {
        await route.fulfill({
          contentType: 'application/json',
          body: JSON.stringify({ result: { files, clone_remote_url: 'https://example.invalid/repo' }, context: {} }),
        })
      } else {
        if (name === 'loadRepoFile_createServerFn_handler') fileRequests += 1
        await route.continue()
      }
    })
    await withPage('/', async (page) => {
      await navigateFromHome(page, repoPath)
      await page.getByText(files.length
        ? 'No README yet. Add README.md or README.html to main and it shows here.'
        : 'Run scope push --main from the CLI to add files to this repository.', { exact: true }).waitFor()
      assert.equal(fileRequests, 0)
    }, { prepare: fulfillTree, viewport })
  }
})

test('central search opens the code at a request and at a past update, then returns to latest', async () => {
  await withPage(requestRepoPath, async (page) => {
    const search = page.getByRole('combobox', { name: 'Search requests, history and files' })
    await waitForClientHydration(search)
    await search.click()
    await page.getByRole('option', { name: /Add bounded retry timing/ }).click()
    await page.waitForURL((url) => url.pathname === `${requestRepoPath}/code` && url.searchParams.get('request') === 'req_demo_ready')
    await page.getByRole('link', { name: 'Open request', exact: true }).waitFor()
    const navigator = page.getByLabel('Repository file navigator')
    await navigator.getByRole('button', { name: 'tests', exact: false }).waitFor()

    await page.getByRole('button', { name: 'Change version', exact: true }).click()
    await page.getByRole('option', { name: /Projected public update/ }).click()
    await page.waitForURL((url) => url.searchParams.has('update'))
    await page.getByRole('link', { name: 'View changes', exact: true }).waitFor()
    await navigator.getByRole('button', { name: 'README.md', exact: true }).waitFor()
    assert.equal(await navigator.getByRole('button', { name: 'docs', exact: false }).count(), 0)

    await page.getByRole('link', { name: 'Back to latest', exact: true }).click()
    await page.waitForURL((url) => url.pathname === `${requestRepoPath}/code` && !url.searchParams.has('update'))
    await navigator.getByRole('button', { name: 'docs', exact: false }).waitFor()
  }, { viewport })
})
