import assert from 'node:assert/strict'
import { test } from 'node:test'
import { chromium } from 'playwright'

const baseUrl = process.env.SCOPE_WEB_BASE_URL ?? process.env.PLAYWRIGHT_BASE_URL ?? 'http://localhost:3000'
const repo = process.env.SCOPE_SMOKE_REPO ?? 'dev/public-demo'
const owner = repo.split('/')[0]

async function withPage(run) {
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
  try { await run(page) } finally { await browser.close() }
}

test('Markdown-only repository opens its introduction and retains direct file links', async () => {
  await withPage(async (page) => {
    await page.goto(`${baseUrl}/${owner}/update-demo`)
    await page.getByRole('tab', { name: 'README.md', exact: true }).waitFor()
    await page.getByRole('tabpanel').locator('article h1').waitFor()
    assert.equal(new URL(page.url()).searchParams.has('file'), false)
    assert.equal(await page.getByRole('alert').count(), 0)
    await page.goto(`${baseUrl}/${repo}?file=src%2Fapp.ts`)
    await page.getByRole('tab', { name: 'src/app.ts', exact: true }).waitFor()
    await page.locator('pre code').filter({ hasText: 'export function greet' }).waitFor()
    await page.goBack()
    await page.getByRole('tab', { name: 'README.md', exact: true }).waitFor()
  })
})

test('file finder supports nested paths, keyboard selection, clearing and mobile shortcut', async () => {
  await withPage(async (page) => {
    await page.goto(`${baseUrl}/${repo}`)
    await page.getByRole('tab', { name: 'README.html', exact: true }).waitFor()
    await page.getByRole('tab', { name: 'README.html', exact: true }).dblclick()
    const finder = page.getByRole('searchbox', { name: 'Find file' })
    await page.getByRole('tab', { name: 'README.html', exact: true }).focus()
    await page.keyboard.press('/')
    await page.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === 'Find file')
    await finder.fill('src/')
    await page.getByRole('list', { name: 'Matching files' }).getByRole('button', { name: 'src/app.ts' }).waitFor()
    await page.keyboard.press('Enter')
    await page.locator('pre code').filter({ hasText: 'export function greet' }).waitFor()
    assert.equal(new URL(page.url()).searchParams.get('file'), 'src/app.ts')
    assert.equal(await page.getByRole('tab', { name: 'README.html', exact: true }).count(), 1)
    assert.equal(await finder.inputValue(), '')
    await finder.fill('does-not-exist')
    await page.getByText('No matching visible files.', { exact: true }).waitFor()
    await page.getByRole('button', { name: 'Clear file search' }).click()
    await page.getByRole('button', { name: 'README.html', exact: true }).waitFor()
    await finder.fill('src')
    await page.keyboard.press('/')
    assert.equal(await finder.inputValue(), 'src/')
    await page.keyboard.press('Escape')
    assert.equal(await finder.inputValue(), '')
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
    await page.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === 'Find file')
    assert.equal(await finder.isVisible(), true)
    await finder.fill('README')
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('Enter')
    await page.getByRole('tab', { name: 'README.html', exact: true }).waitFor()
    assert.equal(await page.getByRole('button', { name: /^files README.html$/ }).getAttribute('aria-expanded'), 'false')
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
  })
})

test('README details preserve inspection without exposing repeated navigator metadata', async () => {
  await withPage(async (page) => {
    await page.goto(`${baseUrl}/${repo}`)
    const preview = page.locator('iframe[title="README.html preview"]')
    await preview.waitFor()
    assert.equal(await preview.getAttribute('sandbox'), '')
    assert.equal(await page.getByLabel('Repository file navigator').getByText('Tracked', { exact: true }).count(), 0)
    assert.equal(await page.getByText('Sandboxed document', { exact: true }).isVisible(), false)
    await page.getByLabel('File details', { exact: true }).click()
    await page.getByText(/^Blob:/).waitFor()
    await page.getByText('Sandboxed document. Repository HTML runs in an isolated preview.', { exact: true }).waitFor()
    await page.keyboard.press('Escape')
    assert.equal(await page.getByText(/^Blob:/).isVisible(), false)
  })
})

function serverFunctionName(request) {
  try {
    const id = new URL(request.url()).pathname.split('/').at(-1)
    return JSON.parse(Buffer.from(id, 'base64url')).export
  } catch { return '' }
}

async function navigateFromHome(page, path) {
  await page.goto(baseUrl)
  await page.waitForFunction(() => globalThis.__TSR_ROUTER__)
  const url = new URL(path, baseUrl)
  await page.evaluate(({ to, search }) => { void globalThis.__TSR_ROUTER__.navigate({ to, search }) }, {
    to: url.pathname,
    search: Object.fromEntries(url.searchParams),
  })
}

test('an explicit file remains readable while its tree is pending and after the tree fails', async () => {
  await withPage(async (page) => {
    let releaseTree
    const held = new Promise((resolve) => { releaseTree = resolve })
    let intercepted = false
    await page.route('**/_serverFn/**', async (route) => {
      if (serverFunctionName(route.request()) === 'loadRepoContent_createServerFn_handler') {
        intercepted = true
        await held
        await route.abort('failed').catch(() => {})
      } else await route.continue()
    })
    try {
      await navigateFromHome(page, `/${repo}?file=src%2Fapp.ts`)
      await page.locator('pre code').filter({ hasText: 'export function greet' }).waitFor()
      assert.equal(intercepted, true)
      releaseTree()
      await page.getByLabel('Repository file navigator').getByRole('alert').waitFor()
      assert.equal(await page.locator('pre code').filter({ hasText: 'export function greet' }).isVisible(), true)
      assert.equal(new URL(page.url()).searchParams.get('file'), 'src/app.ts')
    } finally { releaseTree() }
  })
})

test('views without a README or any files open deliberately without guessing a file request', async () => {
  for (const files of [
    [{ path: '/src/app.ts', oid: 'test-oid', tracked: true, visibility: 'Public' }],
    [],
  ]) {
    await withPage(async (page) => {
      let fileRequests = 0
      await page.route('**/_serverFn/**', async (route) => {
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
      await navigateFromHome(page, `/${repo}`)
      await page.getByText(files.length
        ? 'No README in this view. Browse the files or use Find file to get started.'
        : 'Run scope push --main from the CLI to add files to this repository.', { exact: true }).waitFor()
      assert.equal(fileRequests, 0)
      assert.equal(await page.getByText('Resources', { exact: true }).count(), 0)
    })
  }
})

test('visible project resources open through file tabs and reopen a closed selected resource', async () => {
  await withPage(async (page) => {
    const files = [
      { path: '/LICENSE', oid: 'license-oid', tracked: true, visibility: 'Public' },
      { path: '/.github/CONTRIBUTING.md', oid: 'contributing-oid', tracked: true, visibility: 'Public' },
      { path: '/internal/notes.md', oid: 'private-oid', tracked: true, visibility: 'Private' },
    ]
    await page.route('**/_serverFn/**', async (route) => {
      const name = serverFunctionName(route.request())
      let result
      if (name === 'loadRepoContent_createServerFn_handler') {
        result = { files, clone_remote_url: 'https://example.invalid/repo' }
      } else if (name === 'loadRepoFile_createServerFn_handler') {
        result = {
          status: 'ready',
          file: {
            ...files[0], size_bytes: 15,
            content: { kind: 'text', text: 'Fixture license' },
          },
        }
      } else {
        await route.continue()
        return
      }
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({ result, context: {} }),
      })
    })
    await navigateFromHome(page, `/${repo}`)
    await page.getByText('Resources', { exact: true }).click()
    await page.getByRole('button', { name: 'Contributing', exact: true }).waitFor()
    assert.equal(await page.getByRole('button', { name: 'Security policy', exact: true }).count(), 0)
    await page.getByRole('button', { name: 'License', exact: true }).click()
    await page.getByRole('tab', { name: 'LICENSE', exact: true }).waitFor()
    await page.locator('pre code').filter({ hasText: 'Fixture license' }).waitFor()
    await page.getByRole('button', { name: 'Close LICENSE', exact: true }).click()
    await page.getByText('Select a file to inspect its contents.', { exact: true }).waitFor()
    await page.getByText('Resources', { exact: true }).click()
    await page.getByRole('button', { name: 'License', exact: true }).click()
    await page.getByRole('tab', { name: 'LICENSE', exact: true }).waitFor()
    await page.locator('pre code').filter({ hasText: 'Fixture license' }).waitFor()
    for (const width of [390, 320]) {
      await page.setViewportSize({ width, height: 844 })
      await page.getByText('Resources', { exact: true }).click()
      const bounds = await page.getByRole('button', { name: 'License', exact: true }).evaluate((element) => {
        const menu = element.parentElement.getBoundingClientRect()
        return { left: menu.left, right: menu.right }
      })
      assert(bounds.left >= 0 && bounds.right <= width, `Resources menu overflowed at ${width}px`)
      await page.keyboard.press('Escape')
    }
  })
})
