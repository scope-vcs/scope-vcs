import assert from 'node:assert/strict'
import test from 'node:test'
import { fixtureClerk, openFixture } from './support/fixture-server.mjs'

async function workspacePage(t) {
  const { base, errors, page } = await openFixture(t, 'request-workspace', { aliases: [fixtureClerk], tailwind: true })
  const railClosed = () => page.locator('.request-workspace-sidebar[data-state="closed"]:not([data-closing])').waitFor()
  return { page, base, errors, railClosed }
}

test('request rows exchange age and actions and restore the selected view across repository sections', async (t) => {
  const { page, base, errors, railClosed } = await workspacePage(t)
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
  await waitOpacity(age, '1')
  await waitOpacity(actions, '0')
  const rowBounds = await row.boundingBox()
  await row.hover()
  await waitOpacity(age, '0')
  await waitOpacity(actions, '1')
  assert.deepEqual(await row.boundingBox(), rowBounds)
  assert.equal(await age.evaluate((node) => getComputedStyle(node).transitionDuration), '0.15s')
  assert.deepEqual(await actions.locator('button[title]').evaluateAll((buttons) => buttons.map((button) => button.title)), ['Snooze', 'Settle'])
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.hover.png` })
  }
  await page.mouse.move(700, 400)
  await waitOpacity(age, '1')
  await waitOpacity(actions, '0')
  await page.getByRole('button', { name: 'Viewing as Private' }).focus()
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

  assert.equal(await requestsLink.getAttribute('href'), requestsPath)
  await requestsLink.click()
  await page.getByText('Select a request', { exact: true }).waitFor()
  await primary.getByRole('link', { name: 'Runs', exact: true }).click()
  assert.equal(await requestsLink.getAttribute('href'), requestsPath)
  await page.goBack()
  await page.goBack()
  await page.getByText('Discussion content', { exact: true }).waitFor()

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

  const sidebar = page.locator('.request-workspace-sidebar')
  const sidebarWidth = (width) => page.waitForFunction((value) =>
    document.querySelector('.request-workspace-sidebar').getBoundingClientRect().width === value, width)
  const avatars = () => page.locator('.request-workspace-needs-you .request-workspace-row-avatar').evaluateAll((nodes) =>
    nodes.map((node) => node.getBoundingClientRect()).map(({ x, y, width }) => ({ x, y, width })))
  const openRail = async () => {
    const bounds = await sidebar.boundingBox()
    await page.mouse.click(bounds.x + 48, bounds.y + bounds.height - 40)
    await sidebarWidth(360)
    assert.equal(await sidebar.getAttribute('data-state'), 'open')
  }
  await page.getByRole('button', { name: 'Collapse requests sidebar' }).click()
  await sidebarWidth(88)
  await railClosed()
  const railX = (await sidebar.boundingBox()).x
  const closedAvatars = await avatars()
  const logo = await page.getByRole('link', { name: 'Scope home' }).locator('img').boundingBox()
  const logoAxis = logo.x + logo.width / 2
  const expand = await page.getByRole('button', { name: 'Expand requests sidebar' }).boundingBox()
  for (const { x, width } of [...closedAvatars, expand]) assert(Math.abs(x + width / 2 - logoAxis) < 0.5, 'the rail column centers on the topbar logo')
  assert.equal(await page.getByRole('button', { name: 'Viewing as Private' }).isVisible(), false)
  await page.locator('.request-workspace-needs-you > h2').hover()
  assert.equal(await page.locator('.request-workspace-rail-hint').textContent(), 'Needs you · 3')
  await page.locator('[data-request-id="request-2"] .request-workspace-row-avatar').hover()
  const hint = page.locator('.request-workspace-rail-hint')
  assert.match(await hint.textContent(), /^Request without available actions/)
  assert.match(await page.locator('[data-request-id="request-2"]').getByRole('link').textContent(), /Your request, Ready to merge/)
  const hintBox = await hint.boundingBox()
  assert(hintBox.x > railX + 88)
  assert(Math.abs(hintBox.y + hintBox.height / 2 - (closedAvatars[2].y + closedAvatars[2].width / 2)) < 1)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.rail-closed.png` })
  }
  await page.mouse.move(700, 400)
  await hint.waitFor({ state: 'detached' })
  await page.waitForFunction(() => getComputedStyle(document.querySelector('.request-workspace-resize-handle')).backgroundColor === 'rgba(0, 0, 0, 0)')
  await page.locator('[data-request-id="request-1"] .request-workspace-row-avatar').click()
  await page.waitForURL(/\/request-1$/)
  assert.equal(await sidebar.getAttribute('data-state'), 'closed')
  await openRail()
  assert.deepEqual(await avatars(), closedAvatars)
  assert.equal((await page.getByRole('searchbox').boundingBox()).x, logo.x)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.rail-open.png` })
  }
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('Animation.enable')
  await cdp.send('Animation.setPlaybackRate', { playbackRate: 0.05 })
  await page.keyboard.press('Escape')
  assert.equal(await sidebar.getAttribute('data-state'), 'open')
  assert.equal(await sidebar.getAttribute('data-closing'), 'true')
  assert.equal(await page.getByRole('searchbox').isVisible(), true)
  await cdp.send('Animation.setPlaybackRate', { playbackRate: 1 })
  await sidebarWidth(88)
  await railClosed()
  await openRail()
  await page.getByRole('button', { name: 'Collapse requests sidebar' }).click()
  await sidebarWidth(88)
  await railClosed()
  await openRail()
  await page.keyboard.press('[')
  await sidebarWidth(88)
  await railClosed()
  await openRail()
  await page.mouse.click(900, 500)
  await sidebarWidth(88)
  await railClosed()
  await openRail()
  await row.getByRole('link').click()
  await page.waitForURL(/\/request-0$/)
  await sidebarWidth(88)
  await railClosed()
  await page.keyboard.press('/')
  await sidebarWidth(360)
  assert.equal(await page.getByRole('searchbox').evaluate((node) => node === document.activeElement), true)
  await page.keyboard.press('Escape')
  await sidebarWidth(88)
  await railClosed()
  await page.getByRole('button', { name: 'Expand requests sidebar' }).click()
  await page.getByRole('button', { name: 'Collapse requests sidebar' }).waitFor()
  assert.equal(await sidebar.getAttribute('data-state'), 'pinned')
  await page.keyboard.press('[')
  await sidebarWidth(88)
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
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.keyboard.press('[')
  await railClosed()
  await page.setViewportSize({ width: 390, height: 844 })
  await page.locator('[data-request-id="request-2"] .request-workspace-row-avatar').hover()
  await hint.waitFor({ state: 'attached' })
  assert.equal(await hint.isVisible(), false, 'phone layouts have no rail, so rail hints stay hidden')
  assert.deepEqual(errors, [])
})

test('the viewing-as picker names the current view and switches views from its menu', async (t) => {
  const { page, base, errors } = await workspacePage(t)
  await page.goto(new URL('/adam/demo/requests', base).href)
  const trigger = page.getByRole('button', { name: 'Viewing as Private' })
  await trigger.hover()
  assert.equal(await page.getByRole('tooltip').textContent(), 'Viewing as Private')
  await trigger.click()
  const menu = page.getByRole('menu', { name: 'Viewing as' })
  assert.deepEqual(await menu.getByRole('menuitemradio').allTextContents(), ['Public', 'Private'])
  assert.equal(await menu.getByRole('menuitemradio', { name: 'Private' }).getAttribute('aria-checked'), 'true')
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.viewing-as-menu.png` })
  }
  await page.keyboard.press('j')
  assert.equal(await menu.getByRole('menuitemradio', { name: 'Private' }).evaluate((node) => node === document.activeElement), true)
  await page.keyboard.press('ArrowUp')
  await page.keyboard.press('Enter')
  const publicTrigger = page.getByRole('button', { name: 'Viewing as Public' })
  await publicTrigger.waitFor()
  assert.equal(await menu.isVisible(), false)
  assert.equal(await publicTrigger.evaluate((node) => node === document.activeElement), true)
  assert.equal(new URL(page.url()).searchParams.get('view'), 'public')

  await page.setViewportSize({ width: 1280, height: 200 })
  await publicTrigger.click()
  await page.waitForFunction((node) => {
    const { bottom, top } = node.getBoundingClientRect()
    return top >= 0 && bottom <= window.innerHeight
  }, await menu.elementHandle())
  assert.deepEqual(errors, [])
})

test('collapsed request loading keeps its header stable without guessing avatar rows', async (t) => {
  const { page, base, errors, railClosed } = await workspacePage(t)
  await page.goto(new URL('/adam/demo/requests/request-0', base).href)
  await page.getByRole('button', { name: 'Collapse requests sidebar' }).click()
  const sidebar = page.getByRole('complementary', { name: 'Requests workspace' })
  await page.waitForFunction(() => document.querySelector('.request-workspace-sidebar').getBoundingClientRect().width === 88)
  await railClosed()
  const header = sidebar.locator('.request-workspace-needs-you > h2')
  const toggle = page.getByRole('button', { name: 'Expand requests sidebar' })
  const geometry = async () => ({
    rail: await sidebar.boundingBox(),
    toggle: await toggle.boundingBox(),
    count: await header.boundingBox(),
  })
  const initial = await geometry()
  const empty = { requests: [], next_cursor: null, next_attention_at_unix: null }
  const done = [0, 1, 2].map((index) => ({
    request: { id: `request-${index}`, title: `Closed request ${index}` },
    author: { handle: 'adam' },
    attention: { group: 'done', reason: 'closed' },
    attention_at_unix: 1_790_000_000,
    next_step: 'done',
  }))
  const pages = { active: empty, unclaimed: empty, set_aside: empty, done: { ...empty, requests: done } }
  for (const maintainer of [null, true]) {
    await page.evaluate((maintainer) => window.setQueue({ pages: undefined, maintainer, loading: true }), maintainer)
    await sidebar.locator('[aria-busy="true"]').waitFor()
    assert.deepEqual(await geometry(), initial)
    const placeholders = sidebar.locator('[data-slot="skeleton"]:visible')
    assert.equal(await placeholders.count(), 1, 'the collapsed rail must not invent avatar slots before the queue is known')
    const placeholder = await placeholders.boundingBox()
    const count = await header.boundingBox()
    assert(placeholder.y >= count.y && placeholder.y + placeholder.height <= count.y + count.height)
    assert.equal(placeholder.x + placeholder.width / 2, initial.rail.x + initial.rail.width / 2, 'the count placeholder stays centered in both pending phases')
    if (maintainer === true) {
      await header.hover()
      assert.equal(await page.locator('.request-workspace-rail-hint').textContent(), 'Needs you')
      await page.mouse.move(700, 400)
    }
    if (process.env.SCOPE_COMPONENT_SCREENSHOT && maintainer === true) {
      await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.rail-pending.png` })
    }
    await toggle.click()
    await page.waitForFunction(() => document.querySelector('.request-workspace-sidebar').getBoundingClientRect().width === 360)
    assert(await sidebar.locator('[data-slot="skeleton"]:visible').count() > 1, 'expanded loading still shows list placeholders')
    await page.getByRole('button', { name: 'Collapse requests sidebar' }).click()
    await page.waitForFunction(() => document.querySelector('.request-workspace-sidebar').getBoundingClientRect().width === 88)
    await railClosed()
  }
  await page.evaluate((pages) => window.setQueue({ pages, maintainer: true, loading: false }), pages)
  const selected = sidebar.getByRole('link', { name: /Closed request 0/ })
  await selected.waitFor()
  assert.deepEqual(await geometry(), initial)
  assert.equal(await header.textContent(), 'Needs you0')
  assert.equal(await selected.getAttribute('aria-current'), 'page')
  assert.equal(await sidebar.locator('.request-workspace-rail-more').count(), 0, 'closed requests stay out of the open count')
  assert.equal(await sidebar.locator('.request-workspace-rail-caught-up').isVisible(), true)
  await header.hover()
  assert.equal(await page.locator('.request-workspace-rail-hint').textContent(), 'Needs you · 0You’re caught up.')
  const loaded = await selected.boundingBox()
  await page.evaluate((pages) => window.setQueue({ pages, maintainer: true, loading: true }), pages)
  await sidebar.locator('[aria-busy="true"]').waitFor()
  assert.deepEqual(await selected.boundingBox(), loaded, 'refresh retains the selected request')
  assert.equal(await sidebar.locator('[data-slot="skeleton"]:visible').count(), 0)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.rail-loaded.png` })
  }
  const waiting = [3, 4].map((index) => ({ ...done[0], request: { id: `request-${index}`, title: `Waiting request ${index}` }, attention: { group: 'waiting', reason: 'waiting' }, next_step: 'wait' }))
  await page.evaluate((pages) => window.setQueue({ pages, maintainer: true, loading: false }), { ...pages, active: { ...empty, requests: waiting } })
  const more = sidebar.getByRole('button', { name: 'Show 2 more requests' })
  assert.equal(await more.textContent(), '+2')
  await more.hover()
  assert.equal(await page.locator('.request-workspace-rail-hint').textContent(), '2 more2 waiting')
  await more.click()
  await sidebar.getByRole('button', { name: /Done/ }).click()
  await sidebar.getByRole('link', { name: /Closed request 1/ }).click()
  await page.waitForURL(/\/request-1$/)
  await page.waitForFunction(() => document.querySelector('.request-workspace-sidebar').getBoundingClientRect().width === 88)
  await railClosed()
  assert.equal(await sidebar.getByRole('link', { name: /Closed request 1/ }).getAttribute('aria-current'), 'page')
  await page.evaluate(() => window.setQueue({ pages: undefined, maintainer: false, loading: true }))
  await sidebar.locator('[aria-busy="true"]').waitFor()
  assert.equal(await sidebar.locator('[data-slot="skeleton"]:visible').count(), 0)
  await page.setViewportSize({ width: 390, height: 844 })
  assert.equal(await sidebar.isVisible(), false)
  await page.getByRole('navigation', { name: 'Primary' }).getByRole('link', { name: 'Requests', exact: true }).click()
  await sidebar.waitFor()
  assert(await sidebar.locator('[data-slot="skeleton"]:visible').count() > 0, 'mobile request lists keep their loading rows')
  assert.deepEqual(errors, [])
})


test('stack context preserves actionable children and row order when parents are outside the visible section or search', async (t) => {
  const { page, base, errors } = await workspacePage(t)
  await page.goto(new URL('/adam/demo/requests', base).href)
  const node = { id: 'parent', name: 'parent', title: 'Safe parent', state: 'Open', stack_status: 'main', stack_depth: 1, head_oid: 'head', current_revision_id: 'revision' }
  const row = (id, parent, group = 'needs_you') => ({ request: { id, title: `Actionable ${id}`, stack_parent: parent, stack_depth: 2 }, author: { handle: 'adam' }, attention: { group, reason: 'authored', can_set_aside: true }, attention_at_unix: 1790000000 })
  const parent = { kind: 'available', request: node }
  const empty = { requests: [], next_cursor: null, next_attention_at_unix: null }
  const requests = [row('child', parent), row('sibling', parent), row('opaque', { kind: 'unavailable' })]
  const pages = { active: { ...empty, requests }, unclaimed: empty, set_aside: { ...empty, requests: [row('parent', null, 'set_aside')] }, done: empty }
  await page.evaluate((pages) => window.setQueue({ pages, maintainer: true, loading: false }), pages)
  const rows = page.locator('.request-workspace-needs-you [data-request-id]')
  await page.locator('[data-request-id="child"]').waitFor()
  assert.deepEqual(await rows.evaluateAll((nodes) => nodes.map((node) => node.dataset.requestId)), ['child', 'sibling', 'opaque'])
  assert.equal(await page.locator('.request-workspace-needs-you > h2').textContent(), 'Needs you3')
  const child = page.locator('[data-request-id="child"]')
  const lines = (id) => page.locator(`[data-request-id="${id}"] .request-workspace-stack > span`).evaluateAll((nodes) => nodes.map((node) => node.dataset.line))
  assert.equal(await child.getByRole('link').count(), 1)
  assert.equal(await child.locator('.sr-only').filter({ hasText: 'Based on Safe parent' }).count(), 1)
  assert.deepEqual(await lines('child'), ['pass', 'elsewhere'])
  assert.deepEqual(await lines('sibling'), ['elsewhere'])
  assert.deepEqual(await lines('opaque'), ['elsewhere'])
  await page.keyboard.press('[')
  await page.locator('.request-workspace-sidebar[data-state="closed"]:not([data-closing])').waitFor()
  assert.equal(await child.locator('.request-workspace-stack > span').first().isVisible(), false)
  await page.keyboard.press('[')
  await page.locator('.request-workspace-sidebar[data-state="pinned"]').waitFor()
  const together = { ...pages, active: { ...empty, requests: [...requests, row('parent', null)] }, set_aside: empty }
  await page.evaluate((pages) => window.setQueue({ pages, maintainer: true, loading: false }), together)
  await page.locator('.request-workspace-needs-you [data-request-id="parent"]').waitFor()
  assert.deepEqual(await rows.evaluateAll((nodes) => nodes.map((node) => node.dataset.requestId)), ['parent', 'child', 'sibling', 'opaque'])
  assert.deepEqual(await lines('parent'), ['down'])
  assert.deepEqual(await lines('child'), ['pass', 'parent'])
  assert.deepEqual(await lines('sibling'), ['parent'])
  const avatarLeft = (id) => page.locator(`[data-request-id="${id}"] .request-workspace-row-avatar`).evaluate((node) => node.getBoundingClientRect().left)
  assert.equal(await avatarLeft('child') - await avatarLeft('parent'), 25)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.locator('.request-workspace-needs-you').screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.stack.png` })
  const deep = ['d0', 'd1', 'd2', 'd3', 'd4', 'd5', 'd6'].map((id, index, ids) => row(id, index ? { kind: 'available', request: { ...node, id: ids[index - 1], title: `Actionable ${ids[index - 1]}` } } : null))
  await page.evaluate((pages) => window.setQueue({ pages, maintainer: true, loading: false }), { ...pages, active: { ...empty, requests: deep }, set_aside: empty })
  await page.locator('[data-request-id="d6"]').waitFor()
  assert.equal(await avatarLeft('d6'), await avatarLeft('d4'))
  assert.deepEqual(await lines('d6'), ['chain'])
  assert.equal(await page.locator('[data-request-id="d6"] .request-workspace-row-link').evaluate((node) => node.scrollWidth <= node.clientWidth && node.closest('.request-workspace-sidebar').getBoundingClientRect().right >= node.getBoundingClientRect().right), true)
  await page.evaluate((pages) => window.setQueue({ pages, maintainer: true, loading: false }), pages)
  const searched = { ...pages, active: { ...empty, requests: [requests[0]] }, set_aside: empty }
  await page.evaluate((pages) => { window.setQuery('child'); window.setQueue({ pages, maintainer: true, loading: false }) }, searched)
  await page.locator('[data-request-id="sibling"]').waitFor({ state: 'detached' })
  assert.equal(await child.isVisible(), true)
  assert.deepEqual(await lines('child'), ['elsewhere'])
  assert.equal(await page.locator('[data-request-id="parent"]').count(), 0)
  assert.deepEqual(await page.locator('.request-workspace-rows [data-request-id]').evaluateAll((nodes) => nodes.map((node) => node.dataset.requestId)), ['child'])
  await page.setViewportSize({ width: 390, height: 844 })
  assert.equal(await child.isVisible(), true)
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.stack-mobile.png`, fullPage: true })
  assert.deepEqual(errors, [])
})
