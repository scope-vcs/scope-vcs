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

test('request rows exchange age and actions and restore the selected view across repository sections', async (t) => {
  const cacheDir = await mkdtemp(join(tmpdir(), 'scope-vite-request-workspace-'))
  t.after(() => rm(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    cacheDir,
    configFile: false,
    root: fileURLToPath(new URL('./fixtures/request-workspace', import.meta.url)),
    plugins: [tailwindcss()],
    server: { host: '127.0.0.1', port: 0, fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] } },
    resolve: { alias: [
      { find: '@clerk/tanstack-react-start', replacement: fileURLToPath(new URL('./fixtures/request-workspace/clerk.tsx', import.meta.url)) },
      { find: '@', replacement: fileURLToPath(new URL('../src', import.meta.url)) },
      ...['react/jsx-dev-runtime', 'react/jsx-runtime', 'react-dom/client', 'react']
        .map((name) => ({ find: name, replacement: require.resolve(name) })),
    ] },
    oxc: { jsx: { runtime: 'automatic' } },
  })
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
  page.setDefaultTimeout(10_000)
  const errors = []
  page.on('pageerror', (error) => errors.push(error.message))
  const base = server.resolvedUrls.local[0]
  const requestsPath = '/adam/demo/requests'
  const row = page.locator('[data-request-id="request-0"]')
  const age = row.locator('time')
  const actions = row.locator('fieldset')
  const primary = page.getByRole('navigation', { name: 'Primary' })
  const requestsLink = primary.getByRole('link', { name: 'Requests', exact: true })

  async function waitOpacity(locator, opacity) {
    await page.waitForFunction(([node, value]) => getComputedStyle(node).opacity === value,
      [await locator.elementHandle(), opacity])
  }
  await page.goto(new URL(requestsPath, base).href)
  await row.waitFor()
  // The workspace shares the topbar's rail, so on wide screens the sidebar
  // starts under the logo instead of at the window edge.
  await page.setViewportSize({ width: 1965, height: 900 })
  const railLeft = (selector) => page.locator(selector).evaluate((node) => node.getBoundingClientRect().left)
  assert.equal(await railLeft('.request-workspace-shell'), await railLeft('.application-topbar > *'))
  assert(await railLeft('.request-workspace-shell') > 0)
  await page.setViewportSize({ width: 1280, height: 900 })
  const title = row.locator('[title]').first()
  assert.deepEqual(await title.evaluate((node) => ({
    nowrap: getComputedStyle(node).whiteSpace,
    overflow: getComputedStyle(node).textOverflow,
    truncated: node.scrollWidth > node.clientWidth,
    fullTitle: node.title === node.textContent,
  })), { nowrap: 'nowrap', overflow: 'ellipsis', truncated: true, fullTitle: true })
  assert.equal(await page.locator('.request-workspace-disclosures').evaluate((node) =>
    getComputedStyle(node).borderTopWidth), '0px')
  assert.equal(await page.locator('.request-workspace-row').last().evaluate((node) =>
    getComputedStyle(node).borderBottomWidth), '1px')
  await waitOpacity(age, '1')
  await waitOpacity(actions, '0')
  const rowBounds = await row.boundingBox()
  await row.hover()
  await waitOpacity(age, '0')
  await waitOpacity(actions, '1')
  assert.deepEqual(await row.boundingBox(), rowBounds)
  assert.equal(await age.evaluate((node) => getComputedStyle(node).transitionDuration), '0.15s')
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.hover.png` })
  }
  await page.mouse.move(700, 400)
  await waitOpacity(age, '1')
  await waitOpacity(actions, '0')
  await page.getByRole('button', { name: 'Collapse requests sidebar' }).focus()
  await page.keyboard.press('Tab')
  assert.equal(await row.getByRole('link').evaluate((node) => node === document.activeElement), true)
  await waitOpacity(age, '0')
  await waitOpacity(actions, '1')
  await row.getByRole('button', { name: 'Snooze request request-0' }).click()
  await page.mouse.move(700, 400)
  await page.getByRole('menu').waitFor()
  await waitOpacity(age, '0')
  await page.keyboard.press('Escape')
  await page.getByRole('searchbox').focus()
  await waitOpacity(age, '1')
  await page.locator('[data-request-id="request-2"]').hover()
  assert.equal(await page.locator('[data-request-id="request-2"] time').evaluate((node) =>
    getComputedStyle(node).opacity), '1')
  await page.mouse.move(700, 400)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.idle.png` })
  }

  await row.getByRole('link').click()
  await page.mouse.move(700, 400)
  await waitOpacity(age, '1')
  await waitOpacity(actions, '0')
  for (const view of ['Changes', 'Discussion']) {
    await page.getByRole('navigation', { name: 'Request views' }).getByRole('link', { name: view }).click()
    const selectedUrl = page.url()
    await primary.getByRole('link', { name: 'Runs', exact: true }).click()
    await page.getByRole('heading', { name: 'Runs', exact: true }).waitFor()
    assert.equal(new URL(await requestsLink.getAttribute('href'), base).href, selectedUrl)
    await requestsLink.click()
    await page.getByText(`${view} content`, { exact: true }).waitFor()
    assert.equal(page.url(), selectedUrl)
    assert.equal(await row.getByRole('link').getAttribute('aria-current'), 'page')
  }
  await page.goBack()
  await page.getByRole('heading', { name: 'Runs', exact: true }).waitFor()
  await page.goForward()
  await page.getByText('Discussion content', { exact: true }).waitFor()

  // The active Requests tab still clears the selection, and Back restores it.
  assert.equal(await requestsLink.getAttribute('href'), requestsPath)
  await requestsLink.click()
  await page.getByText('Select a request', { exact: true }).waitFor()
  await primary.getByRole('link', { name: 'Runs', exact: true }).click()
  assert.equal(await requestsLink.getAttribute('href'), requestsPath)
  await page.goBack()
  await page.goBack()
  await page.getByText('Discussion content', { exact: true }).waitFor()

  // Access and viewer changes on Runs must discard the previous request link.
  for (const [setter, value] of [['setActor', 'Member'], ['setViewer', 'another-viewer']]) {
    await primary.getByRole('link', { name: 'Runs', exact: true }).click()
    await page.evaluate(([name, next]) => window[name](next), [setter, value])
    await page.waitForFunction((path) => document.querySelector('nav[aria-label="Primary"] a[href$="/requests"]')?.getAttribute('href') === path, requestsPath)
    await requestsLink.click()
    await page.getByText('Select a request', { exact: true }).waitFor()
    await row.getByRole('link').click()
  }
  await page.evaluate(() => window.navigate('/adam/other/runs'))
  await page.getByRole('heading', { name: 'Runs', exact: true }).waitFor()
  assert.equal(await requestsLink.getAttribute('href'), '/adam/other/requests')
  await page.evaluate(() => window.navigate('/adam/demo/requests'))
  await page.getByText('Select a request', { exact: true }).waitFor()
  await primary.getByRole('link', { name: 'Runs', exact: true }).click()
  await requestsLink.click()
  await page.getByText('Select a request', { exact: true }).waitFor()

  // Collapsed, the sidebar is a rail of avatars centred 27px in. Opening widens
  // the same list over the page, so the avatars stay exactly where they were.
  const sidebar = page.locator('.request-workspace-sidebar')
  const sidebarWidth = (width) => page.waitForFunction((value) =>
    document.querySelector('.request-workspace-sidebar').getBoundingClientRect().width === value, width)
  // The rail keeps its open layout while it slides shut and closes after.
  const railClosed = () => page.locator('.request-workspace-sidebar[data-state="closed"]:not([data-closing])').waitFor()
  const avatars = () => page.locator('.request-workspace-needs-you .request-workspace-row-avatar').evaluateAll((nodes) =>
    nodes.map((node) => node.getBoundingClientRect()).map(({ x, y, width }) => ({ x, y, width })))
  const openRail = async () => {
    const bounds = await sidebar.boundingBox()
    await page.mouse.click(bounds.x + 27, bounds.y + bounds.height - 40)
    await sidebarWidth(360)
    assert.equal(await sidebar.getAttribute('data-state'), 'open')
  }
  await page.getByRole('button', { name: 'Collapse requests sidebar' }).click()
  await sidebarWidth(54)
  await railClosed()
  const railX = (await sidebar.boundingBox()).x
  const closedAvatars = await avatars()
  assert.deepEqual(closedAvatars.map(({ x, width }) => x + width / 2 - railX), [27, 27, 27])
  // The caret that expands the sidebar sits on the rail where search was.
  const expand = await page.getByRole('button', { name: 'Expand requests sidebar' }).boundingBox()
  assert.equal(expand.x + expand.width / 2 - railX, 27)
  // An avatar names its request beside the rail, level with the avatar.
  await page.locator('[data-request-id="request-2"] .request-workspace-row-avatar').hover()
  const hint = page.locator('.request-workspace-rail-hint')
  assert.match(await hint.textContent(), /^Request without available actions/)
  const hintBox = await hint.boundingBox()
  assert(hintBox.x > railX + 54)
  assert(Math.abs(hintBox.y + hintBox.height / 2 - (closedAvatars[2].y + closedAvatars[2].width / 2)) < 1)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.rail-closed.png` })
  }
  await page.mouse.move(700, 400)
  await hint.waitFor({ state: 'detached' })
  await page.locator('[data-request-id="request-1"] .request-workspace-row-avatar').click()
  await page.waitForURL(/\/request-1$/)
  assert.equal(await sidebar.getAttribute('data-state'), 'closed')
  await openRail()
  assert.deepEqual(await avatars(), closedAvatars)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.rail-open.png` })
  }
  // Slowed down, the closing rail still shows its list over the page.
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('Animation.enable')
  await cdp.send('Animation.setPlaybackRate', { playbackRate: 0.05 })
  await page.keyboard.press('Escape')
  assert.equal(await sidebar.getAttribute('data-state'), 'open')
  assert.equal(await sidebar.getAttribute('data-closing'), 'true')
  assert.equal(await page.getByRole('searchbox').isVisible(), true)
  await cdp.send('Animation.setPlaybackRate', { playbackRate: 1 })
  await sidebarWidth(54)
  await railClosed()
  // The open rail's caret folds it back to the rail instead of pinning it.
  await openRail()
  await page.getByRole('button', { name: 'Collapse requests sidebar' }).click()
  await sidebarWidth(54)
  await railClosed()
  await openRail()
  await page.keyboard.press('[')
  await sidebarWidth(54)
  await railClosed()
  await openRail()
  await page.mouse.click(900, 500)
  await sidebarWidth(54)
  await railClosed()
  await openRail()
  await row.getByRole('link').click()
  await page.waitForURL(/\/request-0$/)
  await sidebarWidth(54)
  await railClosed()
  await page.keyboard.press('/')
  await sidebarWidth(360)
  assert.equal(await page.getByRole('searchbox').evaluate((node) => node === document.activeElement), true)
  await page.keyboard.press('Escape')
  await sidebarWidth(54)
  await railClosed()
  await page.getByRole('button', { name: 'Expand requests sidebar' }).click()
  await page.getByRole('button', { name: 'Collapse requests sidebar' }).waitFor()
  assert.equal(await sidebar.getAttribute('data-state'), 'pinned')
  await page.keyboard.press('[')
  await sidebarWidth(54)
  await railClosed()
  await page.keyboard.press('[')
  await page.getByRole('button', { name: 'Collapse requests sidebar' }).waitFor()
  assert.equal(await sidebar.getAttribute('data-state'), 'pinned')
  await requestsLink.click()
  await page.getByText('Select a request', { exact: true }).waitFor()

  await page.emulateMedia({ reducedMotion: 'reduce' })
  assert(await age.evaluate((node) => parseFloat(getComputedStyle(node).transitionDuration) <= 0.00001))
  await page.setViewportSize({ width: 390, height: 844 })
  await waitOpacity(age, '0')
  await waitOpacity(actions, '1')
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
  assert.equal(await page.locator('[data-request-id="request-2"] time').evaluate((node) =>
    getComputedStyle(node).opacity), '1')
  const label = await row.locator('.request-workspace-row-meta > span').boundingBox()
  const buttons = await actions.boundingBox()
  assert(label.x + label.width < buttons.x)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.mobile.png` })
  }
  assert.deepEqual(errors, [])
})
