import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import tailwindcss from '@tailwindcss/vite'

const require = createRequire(import.meta.url)

test('same-view stack comparisons render child layers at the parent pin without parent file leakage', async (t) => {
  const cacheDir = await mkdtemp(join(tmpdir(), 'scope-vite-request-stack-'))
  t.after(() => rm(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    cacheDir, configFile: false,
    root: fileURLToPath(new URL('./fixtures/request-stack', import.meta.url)),
    plugins: [tailwindcss()],
    server: { host: '127.0.0.1', port: 0, fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] } },
    resolve: { alias: [
      { find: '@', replacement: fileURLToPath(new URL('../src', import.meta.url)) },
      ...['react/jsx-dev-runtime', 'react/jsx-runtime', 'react-dom/client', 'react']
        .map((name) => ({ find: name, replacement: require.resolve(name) })),
    ] },
    oxc: { jsx: { runtime: 'automatic' } },
  })
  t.after(() => server.close())
  await server.listen()
  await server.environments.client.warmupRequest('/entry.tsx')
  await server.waitForRequestsIdle()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
  page.setDefaultTimeout(10_000)
  async function captureComparison(name) {
    for (const [viewport, size] of [
      ['desktop', { width: 1280, height: 900 }],
      ['mobile', { width: 390, height: 844 }],
    ]) {
      await page.setViewportSize(size)
      if (viewport === 'mobile') {
        const navigator = page.getByRole('button', { name: 'files src/child.ts', exact: true })
        if (await navigator.getAttribute('aria-expanded') === 'false') await navigator.click()
      }
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
      const file = page.getByRole('button', { name: 'child.ts', exact: true })
      assert.equal(await file.isVisible(), true)
      const bounds = await file.boundingBox()
      assert.ok(bounds && bounds.x >= 0 && bounds.x + bounds.width <= size.width)
      if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
        await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.${name}.${viewport}.png`, fullPage: true })
      }
    }
  }
  const errors = []
  page.on('pageerror', (error) => errors.push(error.message))
  await page.goto(new URL('/adam/demo/requests/child/changes', server.resolvedUrls.local[0]).href)
  const dependency = page.getByRole('region', { name: 'Request dependency' })
  const parent = dependency.getByRole('link', { name: 'Parent foundation' })
  assert.equal(await parent.getAttribute('href'), '/adam/demo/requests/parent')
  assert.equal(await dependency.locator('[title="parent-revision"]').textContent(), 'Pinned revision parent-revision · aaaaaaaaaaaa')
  assert.equal(await dependency.locator('[title="current-parent-revision"]').textContent(), 'Current revision current-parent-revision · cccccccccccc')
  assert.equal(await dependency.getByRole('link', { name: 'Follow-up child' }).getAttribute('href'), '/adam/demo/requests/grandchild')
  assert.equal(await page.getByRole('button', { name: 'Merge', exact: true }).isDisabled(), true)
  assert.equal(await page.getByRole('button', { name: 'Request layer', exact: true }).getAttribute('aria-current'), 'page')
  const revisionReference = page.getByRole('link', { name: 'Review the child revision Open', exact: true })
  await revisionReference.waitFor()
  assert.equal(new URL(await revisionReference.getAttribute('href'), page.url()).searchParams.get('discussion'), 'layer-reference')
  assert.equal(await page.getByRole('link', { name: 'Review the child commit Open', exact: true }).count(), 0)
  assert.equal(await page.evaluate(() => window.referenceCalls.length), 1)
  assert.equal((await page.evaluate(() => window.referenceCalls[0])).revision_id, 'child-revision')
  assert.equal((await page.evaluate(() => window.referenceCalls[0])).commit_oid, undefined)
  await page.getByRole('button', { name: 'child.ts', exact: true }).click()
  await page.waitForFunction(() => window.calls.length === 1)
  assert.equal((await page.evaluate(() => window.calls[0])).comparison, 'request')
  await page.getByRole('button', { name: 'Push changes', exact: true }).click()
  await revisionReference.waitFor()
  assert.equal(await page.evaluate(() => window.referenceCalls.length), 1)
  await page.getByRole('button', { name: 'child.ts', exact: true }).click()
  await page.waitForFunction(() => window.calls.length === 2)
  assert.equal((await page.evaluate(() => window.calls[1])).comparison, 'push')
  await page.getByRole('button', { name: 'Request layer', exact: true }).click()
  await page.getByRole('button', { name: 'child.ts', exact: true }).click()
  assert.equal(await page.evaluate(() => window.calls.length), 2)
  await page.getByRole('button', { name: 'Commits', exact: true }).click()
  await page.getByRole('button', { name: 'child.ts', exact: true }).click()
  await page.waitForFunction(() => window.calls.length === 3)
  const commit = await page.evaluate(() => window.calls[2])
  assert.equal(commit.comparison, 'commit')
  assert.equal(commit.commit_oid, 'b'.repeat(40))
  assert.equal(commit.path, '/src/child.ts')
  assert.equal(await page.getByRole('heading', { name: 'Add child UI', exact: true }).isVisible(), true)
  await page.getByRole('link', { name: 'Review the child commit Open', exact: true }).waitFor()
  assert.equal(await revisionReference.count(), 1)
  assert.equal(await page.evaluate(() => window.referenceCalls.length), 2)
  assert.equal((await page.evaluate(() => window.referenceCalls[1])).commit_oid, 'b'.repeat(40))
  assert.equal(await page.getByText('No file changes in this commit.', { exact: true }).count(), 0)
  await captureComparison('private-commit')
  await page.setViewportSize({ width: 1280, height: 900 })
  for (const [status, label] of [
    ['parent_extended', 'Parent extended'], ['parent_rewritten', 'Parent rewritten'],
    ['parent_closed', 'Parent closed'], ['ancestor_blocked', 'Ancestor blocked'],
    ['integration_conflict', 'Integration conflict'],
  ]) {
    await page.evaluate((value) => window.setStackStatus(value), status)
    await dependency.getByText(`· ${label}`, { exact: true }).waitFor()
    assert.equal(await parent.getAttribute('href'), '/adam/demo/requests/parent')
    assert.equal(await page.getByRole('button', { name: 'Merge', exact: true }).isDisabled(), true)
  }
  await page.setViewportSize({ width: 390, height: 844 })
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  const selector = dependency.getByRole('combobox', { name: 'Navigate stack' })
  assert.equal(await selector.isVisible(), true)
  assert.deepEqual(await selector.locator('option').allTextContents(), ['Child UI · depth 2', 'Ancestor: Parent foundation', 'Child: Follow-up child'])
  await page.evaluate(() => window.setChildrenError('Children are offline.'))
  await dependency.getByRole('alert').getByText('Children are offline.', { exact: true }).waitFor()
  const retry = dependency.getByRole('button', { name: 'Retry children', exact: true })
  assert.equal(await retry.isVisible(), true)
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.child-retry.mobile.png`, fullPage: true })
  await retry.click()
  assert.deepEqual(await page.evaluate(() => window.calls.at(-1)), { kind: 'children' })
  await page.evaluate(() => window.setChildrenError(null))
  await dependency.getByRole('alert').waitFor({ state: 'detached' })
  assert.equal(await dependency.getByRole('alert').count(), 0)
  await page.evaluate(() => window.setOpaque(true))
  await dependency.locator('p').filter({ hasText: 'Unavailable dependency' }).waitFor()
  assert.equal(await dependency.getByRole('link', { name: 'Parent foundation' }).count(), 0)
  assert.equal(await dependency.locator('[title="parent-revision"]').count(), 1)
  assert.equal(await dependency.locator('[title="current-parent-revision"]').count(), 0)
  assert.equal(await dependency.getByText('Pinned revision parent-revision · aaaaaaaaaaaa', { exact: true }).isVisible(), true)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.unavailable-parent.mobile.png`, fullPage: true })
  for (const view of ['public', 'restricted']) {
    await page.setViewportSize({ width: 1280, height: 900 })
    await page.goto(new URL(`/adam/demo/requests/child/changes?view=${view}`, server.resolvedUrls.local[0]).href)
    const visibleDependency = page.getByRole('region', { name: 'Request dependency' })
    assert.equal(await visibleDependency.getByRole('link', { name: 'Parent foundation' }).getAttribute('href'), `/adam/demo/requests/parent?view=${view}`)
    assert.equal(await visibleDependency.locator('[title="parent-revision"]').textContent(), 'Pinned revision parent-revision · aaaaaaaaaaaa')
    assert.equal(await page.getByRole('button', { name: 'Request layer', exact: true }).getAttribute('aria-current'), 'page')
  const revisionReference = page.getByRole('link', { name: 'Review the child revision Open', exact: true })
  await revisionReference.waitFor()
  assert.equal(new URL(await revisionReference.getAttribute('href'), page.url()).searchParams.get('discussion'), 'layer-reference')
  assert.equal(await page.getByRole('link', { name: 'Review the child commit Open', exact: true }).count(), 0)
  assert.equal(await page.evaluate(() => window.referenceCalls.length), 1)
  assert.equal((await page.evaluate(() => window.referenceCalls[0])).revision_id, 'child-revision')
  assert.equal((await page.evaluate(() => window.referenceCalls[0])).commit_oid, undefined)
    assert.equal(await page.getByRole('button', { name: 'parent.ts', exact: true }).count(), 0)
    await page.getByRole('button', { name: 'child.ts', exact: true }).click()
    await page.waitForFunction(() => window.calls.length === 1)
    const layerDiff = await page.evaluate(() => window.calls[0])
    assert.equal(layerDiff.comparison, 'request')
    assert.equal(layerDiff.commit_oid, undefined)
    assert.equal(layerDiff.revision_id, 'child-revision')
    assert.equal(layerDiff.path, '/src/child.ts')
    if (view === 'public') await captureComparison('public-layer')
    await page.setViewportSize({ width: 1280, height: 900 })
    await page.getByRole('button', { name: 'Push changes', exact: true }).click()
    assert.equal(await page.getByRole('button', { name: 'parent.ts', exact: true }).count(), 0)
    await page.getByRole('button', { name: 'child.ts', exact: true }).click()
    await page.waitForFunction(() => window.calls.length === 2)
    const pushDiff = await page.evaluate(() => window.calls[1])
    assert.equal(pushDiff.comparison, 'push')
    assert.equal(pushDiff.commit_oid, undefined)
    assert.equal(pushDiff.revision_id, 'child-revision')
    if (view === 'public') await captureComparison('public-push')
    await page.setViewportSize({ width: 1280, height: 900 })
    await page.getByRole('button', { name: 'Commits', exact: true }).click()
    assert.equal(await page.getByRole('button', { name: 'parent.ts', exact: true }).isVisible(), true)
    await page.getByRole('button', { name: 'child.ts', exact: true }).click()
    await page.waitForFunction(() => window.calls.length === 3)
    assert.equal((await page.evaluate(() => window.calls[2])).comparison, 'commit')
    await page.getByRole('button', { name: 'Request layer', exact: true }).click()
    await page.getByRole('button', { name: 'child.ts', exact: true }).click()
    assert.equal(await page.evaluate(() => window.calls.length), 3)
    assert.equal(await page.getByRole('button', { name: 'parent.ts', exact: true }).count(), 0)
  }
  assert.deepEqual(errors, [])
})
