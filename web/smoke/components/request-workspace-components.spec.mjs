import assert from 'node:assert/strict'
import test from 'node:test'
import { fixtureClerk, openFixture } from '../support/fixture-server.mjs'

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
  const diffStat = row.locator('.request-workspace-row-diff-stat')
  const diffStats = () => page.locator('.request-workspace-row').evaluateAll((rows) => rows.map((node) => {
    const stat = node.querySelector('.request-workspace-row-diff-stat')
    if (!stat || getComputedStyle(stat).display === 'none') return null
    const statBox = stat.getBoundingClientRect()
    const label = node.querySelector('.request-workspace-row-meta > span:first-child')
    return {
      text: stat.querySelector('[aria-hidden="true"]').textContent,
      title: stat.title,
      clear: label.getBoundingClientRect().right <= statBox.left
        && statBox.right <= node.querySelector('time').getBoundingClientRect().left,
    }
  }))
  assert.deepEqual(await diffStats(), [
    { text: '+2.4k −41', title: '2,480 additions, 41 deletions across 6 files', clear: true },
    { text: '3 files', title: '3 files changed, no line changes', clear: true },
    null,
  ])
  const widthHandle = page.getByRole('separator', { name: 'Requests sidebar width' })
  await widthHandle.focus()
  await page.keyboard.press('ArrowLeft')
  await page.keyboard.press('ArrowLeft')
  await page.keyboard.press('ArrowLeft')
  await page.waitForFunction(() => document.querySelector('.request-workspace-sidebar').getBoundingClientRect().width === 330)
  assert.deepEqual(await diffStats(), [null, null, null])
  await page.keyboard.press('End')
  await page.waitForFunction(() => document.querySelector('.request-workspace-sidebar').getBoundingClientRect().width === 360)
  await page.mouse.move(700, 400)
  await page.getByRole('searchbox').focus()
  await waitOpacity(diffStat, '1')
  const rowBounds = await row.boundingBox()
  await row.hover()
  await waitOpacity(age, '0')
  await waitOpacity(diffStat, '0')
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
  await waitOpacity(diffStat, '0')
  await waitOpacity(actions, '1')
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
  assert.equal(await page.locator('[data-request-id="request-2"] time').evaluate((node) =>
    getComputedStyle(node).opacity), '1')
  const label = await row.locator('.request-workspace-row-meta > span:first-child').boundingBox()
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

test('pressing a woke row acknowledges it once and custom snooze confirms a chosen time', async (t) => {
  const { page, base, errors } = await workspacePage(t)
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto(new URL('/adam/demo/requests', base).href)
  const row = page.locator('[data-request-id="request-1"]')
  await row.getByRole('link').waitFor()
  await page.evaluate(() => {
    const [first, second, third] = window.fixtureItems
    const empty = { requests: [], next_cursor: null, next_attention_at_unix: null }
    const woke = { ...second, attention: { ...second.attention, reason: 'snooze_expired' } }
    window.setQueue({ pages: { active: { ...empty, requests: [first, woke, third] }, unclaimed: empty, set_aside: empty, done: empty }, maintainer: true, loading: false })
  })
  await row.getByText('Woke').waitFor()
  await page.locator('[data-request-id="request-0"]').getByRole('link').click()
  assert.deepEqual(await page.evaluate(() => window.attentionCommands), [])
  await row.getByRole('link').click()
  assert.deepEqual(await page.evaluate(() => window.attentionCommands), [{ id: 'request-1', action: 'acknowledge' }])

  await page.evaluate(() => window.attentionCommands.splice(0))
  await page.locator('[data-request-id="request-0"]').hover()
  await page.getByRole('button', { name: 'Snooze request request-0' }).click()
  await page.getByRole('menuitem', { name: 'Custom…' }).click()
  const dialog = page.getByRole('dialog', { name: 'Custom snooze' })
  await dialog.waitFor()
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.getByRole('button', { name: /^Date/ }).click()
    await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.custom-snooze.png` })
    await page.getByRole('button', { name: /^Date/ }).click()
  }
  await dialog.getByLabel('Time').fill('00:00')
  await dialog.getByRole('button', { name: 'Snooze', exact: true }).click()
  await dialog.getByRole('alert').getByText('Choose a valid date and time in the future.').waitFor()
  await dialog.getByRole('radio', { name: 'Duration' }).click()
  await dialog.getByLabel('Snooze for').fill('0')
  await dialog.getByRole('button', { name: 'Snooze', exact: true }).click()
  await dialog.getByRole('alert').getByText('Enter a positive duration.').waitFor()
  await dialog.getByRole('button', { name: 'Increase duration' }).click()
  await dialog.getByLabel('Unit').selectOption('days')
  const before = Math.floor(Date.now() / 1_000)
  await dialog.getByRole('button', { name: 'Snooze', exact: true }).click()
  await dialog.waitFor({ state: 'detached' })
  const [{ id, action, until_unix: until }] = await page.evaluate(() => window.attentionCommands)
  assert.deepEqual([id, action], ['request-0', 'snooze'])
  assert(until >= before + 86_400 && until <= Math.floor(Date.now() / 1_000) + 86_400)
  assert.deepEqual(errors, [])
})

test('constrained menus keep keyboard targets visible and stay open while scrolling', async (t) => {
  const { page, base, errors } = await workspacePage(t)
  await page.setViewportSize({ width: 1280, height: 150 })
  await page.goto(new URL('/adam/demo/requests', base).href)
  await page.getByRole('button', { name: 'Snooze request request-0' }).focus()
  await page.keyboard.press('Enter')
  const menu = page.getByRole('menu', { name: 'Snooze request request-0' })
  await menu.waitFor()
  assert.equal(await menu.evaluate((node) => node.scrollHeight > node.clientHeight), true)
  for (const [key, item] of [['End', menu.getByRole('menuitem').last()], ['Home', menu.getByRole('menuitem').first()]]) {
    await page.keyboard.press(key)
    assert.equal(await item.evaluate((node) => {
      const bounds = node.getBoundingClientRect()
      const menuBounds = node.parentElement.getBoundingClientRect()
      return node === document.activeElement && bounds.top >= menuBounds.top && bounds.bottom <= menuBounds.bottom
    }), true, 'keyboard navigation must reveal the focused item')
  }
  const scrollTop = await menu.evaluate((node) => node.scrollTop)
  await menu.hover()
  await page.mouse.wheel(0, 200)
  await page.waitForFunction((top) => document.querySelector(':popover-open')?.scrollTop > top, scrollTop)
  assert.equal(await menu.isVisible(), true)
  await page.keyboard.press('Escape')
  assert.equal(await page.getByRole('button', { name: 'Snooze request request-0' }).evaluate((node) => node === document.activeElement), true)
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
  const row = (id, parent, group = 'needs_you') => ({ request: { id, title: `Actionable ${id}`, stack_parent: parent, stack_depth: 2 }, author: { handle: 'adam' }, attention: { group, reason: 'authored', can_set_aside: true }, attention_at_unix: 1790000000, next_step: 'review' })
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

const stackHintItems = (hint) => hint.locator('li').evaluateAll((items) => items.map((item) => [
  item.querySelector('.request-workspace-rail-hint-title').textContent,
  item.querySelector('[data-step]')?.dataset.step,
  item.querySelector('.request-workspace-rail-hint-step-slot').title,
  item.querySelector('.request-workspace-rail-hint-age')?.textContent ?? null,
  Number(item.style.getPropertyValue('--stack-depth')),
  item.hasAttribute('data-hovered'),
]))

test('the collapsed rail shows a request\'s whole stack on hover while the stack is expanded', async (t) => {
  const { page, base, errors, railClosed } = await workspacePage(t)
  await page.goto(new URL('/adam/demo/requests', base).href)
  const node = { id: 'base', name: 'base', title: 'Stacked base', state: 'Open', stack_status: 'main', stack_depth: 1, head_oid: 'head', current_revision_id: 'revision' }
  const row = (id, parent, extra = {}) => ({
    request: { id, title: `Stacked ${id}`, stack_parent: parent && { kind: 'available', request: { ...node, id: parent, title: `Stacked ${parent}` } }, stack_depth: 1 },
    author: { handle: 'adam' },
    attention: { group: 'needs_you', reason: 'authored', can_set_aside: true },
    attention_at_unix: 1790000000,
    next_step: 'review',
    ...extra,
  })
  const empty = { requests: [], next_cursor: null, next_attention_at_unix: null }
  const requests = [row('base', null), row('first', 'base'), row('ready', 'first', { next_step: 'merge' }), row('plain', null)]
  await page.evaluate((pages) => window.setQueue({ pages, maintainer: true, loading: false }), { active: { ...empty, requests }, unclaimed: empty, set_aside: empty, done: empty })
  await page.locator('[data-request-id="ready"]').waitFor()
  await page.keyboard.press('[')
  await railClosed()
  await page.waitForFunction(() => document.querySelector('.request-workspace-sidebar').getBoundingClientRect().width === 88)
  const avatar = page.locator('[data-request-id="ready"] .request-workspace-row-avatar')
  await avatar.hover()
  const hint = page.locator('.request-workspace-rail-hint')
  assert.deepEqual(await stackHintItems(hint), [
    ['Stacked base', 'review', 'Your request', null, 0, false],
    ['Stacked first', 'review', 'Your request', null, 1, false],
    ['Stacked ready', 'merge', 'Ready to merge', 'Sep 21', 2, true],
  ])
  const title = await hint.locator('li[data-hovered] .request-workspace-rail-hint-title').boundingBox()
  const target = await avatar.boundingBox()
  assert(Math.abs(title.y + title.height / 2 - (target.y + target.height / 2)) < 1, 'the hovered request lines up with its avatar')
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.stack-rail-expanded-hint.png` })
  const baseLink = hint.getByRole('link', { name: /^Stacked base/, includeHidden: true })
  const destination = await baseLink.boundingBox()
  const handle = await page.locator('.request-workspace-resize-handle').boundingBox()
  const avatarCenter = target.y + target.height / 2
  const path = []
  await page.clock.install()
  for (let x = target.x + target.width / 2; x <= destination.x + 20; x += 1) {
    await page.mouse.move(x, avatarCenter)
    const region = await page.evaluate(([x, y]) => {
      const over = document.elementFromPoint(x, y)
      if (over?.closest('.request-workspace-resize-handle')) return 'handle'
      return over?.closest('.request-workspace-rail-hint') ? 'hint' : 'rail'
    }, [x, avatarCenter])
    if (region === path.at(-1)?.region) continue
    path.push({ region, x })
    await page.clock.runFor(1_000)
  }
  assert.deepEqual(path.map(({ region }) => region), ['rail', 'handle', 'hint'], 'the path to the hint crosses the resize handle')
  assert(Math.abs(path[1].x - handle.x) < 8)
  await page.mouse.move(destination.x + 20, destination.y + destination.height / 2, { steps: 12 })
  assert.equal(await hint.locator('li[data-hovered] .request-workspace-rail-hint-title').textContent(), 'Stacked ready')
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForURL(/\/requests\/base$/)
  await hint.waitFor({ state: 'detached' })
  assert.equal(await page.locator('.request-workspace-sidebar').getAttribute('data-state'), 'closed')
  assert.equal(await page.locator('[data-request-id="base"] .request-workspace-row-link').getAttribute('aria-current'), 'page')

  await avatar.hover()
  await page.mouse.move(700, 400, { steps: 8 })
  await hint.waitFor({ state: 'detached' })
  await page.locator('[data-request-id="plain"] .request-workspace-row-avatar').hover()
  assert.deepEqual(await stackHintItems(hint), [['Stacked plain', 'review', 'Your request', 'Sep 21', 0, true]])
  assert.equal(await hint.locator('a').count(), 0)
  assert.equal(await hint.evaluate((node) => getComputedStyle(node).pointerEvents), 'none')
  assert.deepEqual(errors, [])
})

test('a stack taller than the window stays inside it and scrolls to every request', async (t) => {
  const { page, base, errors, railClosed } = await workspacePage(t)
  await page.setViewportSize({ width: 1280, height: 360 })
  await page.goto(new URL('/adam/demo/requests', base).href)
  const node = { name: 'node', state: 'Open', stack_status: 'main', stack_depth: 1, head_oid: 'head', current_revision_id: 'revision' }
  const ids = Array.from({ length: 16 }, (_, index) => `layer-${index}`)
  const requests = ids.map((id, index) => ({
    request: { id, title: `Layer ${index}`, stack_parent: index > 0 && { kind: 'available', request: { ...node, id: ids[index - 1], title: `Layer ${index - 1}` } }, stack_depth: 1 },
    author: { handle: 'adam' },
    attention: { group: 'needs_you', reason: 'authored', can_set_aside: true },
    attention_at_unix: 1790000000,
    next_step: 'review',
  }))
  const empty = { requests: [], next_cursor: null, next_attention_at_unix: null }
  await page.evaluate((pages) => window.setQueue({ pages, maintainer: true, loading: false }), { active: { ...empty, requests }, unclaimed: empty, set_aside: empty, done: empty })
  await page.locator('[data-request-id="layer-0"]').waitFor()
  await page.keyboard.press('[')
  await railClosed()
  await page.waitForFunction(() => document.querySelector('.request-workspace-sidebar').getBoundingClientRect().width === 88)
  const avatar = page.locator('[data-request-id="layer-0"] .request-workspace-row-avatar')
  await avatar.hover()
  const hint = page.locator('.request-workspace-rail-hint')
  assert.equal(await hint.locator('li').count(), 16)
  const placement = await hint.evaluate((node) => ({ ...node.getBoundingClientRect().toJSON(), scrollable: node.scrollHeight > node.clientHeight }))
  assert(placement.top >= 8 && placement.bottom <= 352 && placement.scrollable, 'the hint fits the window and scrolls')
  const target = await avatar.boundingBox()
  await page.mouse.move(placement.left + 60, target.y + target.height / 2, { steps: 20 })
  await page.mouse.wheel(0, 2000)
  const last = hint.getByRole('link', { name: /^Layer 15/, includeHidden: true })
  await page.waitForFunction(() => {
    const hint = document.querySelector('.request-workspace-rail-hint')
    return hint.scrollTop + hint.clientHeight >= hint.scrollHeight - 1
  })
  const destination = await last.boundingBox()
  assert(destination.y + destination.height <= 352, 'the last request scrolls into the window')
  await page.mouse.click(destination.x + 40, destination.y + destination.height / 2)
  await page.waitForURL(/\/requests\/layer-15$/)
  assert.deepEqual(errors, [])
})

test('stacks fold behind their base request and show their details only on hover', async (t) => {
  const { page, base, errors } = await workspacePage(t)
  await page.goto(new URL('/adam/demo/requests', base).href)
  const node = { id: 'base', name: 'base', title: 'Stacked base', state: 'Open', stack_status: 'main', stack_depth: 1, head_oid: 'head', current_revision_id: 'revision' }
  const row = (id, parent, extra = {}) => ({
    request: { id, title: `Stacked ${id}`, stack_parent: parent && { kind: 'available', request: { ...node, id: parent, title: `Stacked ${parent}` } }, stack_depth: 1 },
    author: { handle: 'adam' },
    attention: { group: 'needs_you', reason: 'authored', can_set_aside: true },
    attention_at_unix: 1790000000,
    next_step: 'review',
    ...extra,
  })
  const empty = { requests: [], next_cursor: null, next_attention_at_unix: null }
  const invited = { group: 'needs_you', reason: 'invited', can_set_aside: true }
  const requests = [row('base', null), row('first', 'base'), row('ready', 'first', { next_step: 'merge' }), row('plain', null, { attention: invited })]
  await page.evaluate((pages) => window.setQueue({ pages, maintainer: true, loading: false }), { active: { ...empty, requests }, unclaimed: empty, set_aside: empty, done: empty })
  const ids = () => page.locator('.request-workspace-needs-you [data-request-id]').evaluateAll((nodes) => nodes.map((node) => node.dataset.requestId))
  const opacity = async (locator, value) => page.waitForFunction(([node, value]) =>
    getComputedStyle(node).opacity === value, [await locator.elementHandle(), value])
  const baseRow = page.locator('[data-request-id="base"]')
  const toggle = baseRow.locator('.request-workspace-stack-toggle')
  await page.locator('[data-request-id="first"] .request-workspace-row-link').click()
  await page.mouse.move(1200, 800)
  assert.deepEqual(await ids(), ['base', 'first', 'ready', 'plain'])
  await opacity(toggle, '0')
  await opacity(baseRow.locator('.request-workspace-row-hover-detail'), '0')
  assert.equal(await page.locator('[data-request-id="plain"] .request-workspace-row-hover-detail').count(), 0)

  await baseRow.hover()
  await opacity(toggle, '1')
  await opacity(baseRow.locator('.request-workspace-row-hover-detail'), '1')
  await baseRow.getByRole('button', { name: 'Collapse 2 stacked requests' }).click()
  await page.locator('[data-request-id="first"]').waitFor({ state: 'detached' })
  assert.deepEqual(await ids(), ['base', 'plain'])
  assert.equal(await toggle.getAttribute('aria-expanded'), 'false')
  assert.equal(await baseRow.getAttribute('data-holds-selection'), 'true')
  const summary = baseRow.locator('.request-workspace-row-hover-detail').last()
  assert.equal((await summary.textContent()).trim(), '· 2 stacked, 1 ready')
  await opacity(summary, '1')
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.locator('.request-workspace-needs-you').screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.stack-folded-hover.png` })
  await page.mouse.move(1200, 800)
  await opacity(summary, '0')
  await opacity(toggle, '0')
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.locator('.request-workspace-needs-you').screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.stack-folded.png` })

  await baseRow.hover()
  await baseRow.getByRole('button', { name: 'Expand 2 stacked requests' }).click()
  await page.locator('[data-request-id="ready"]').waitFor()
  assert.deepEqual(await ids(), ['base', 'first', 'ready', 'plain'])
  assert.equal(await baseRow.getAttribute('data-holds-selection'), null)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.locator('.request-workspace-needs-you').screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.stack-expanded-hover.png` })

  await baseRow.getByRole('button', { name: 'Collapse 2 stacked requests' }).click()
  await baseRow.getByRole('button', { name: 'Expand 2 stacked requests' }).click()
  await page.waitForTimeout(300)
  assert.deepEqual(await ids(), ['base', 'first', 'ready', 'plain'])
  assert.equal(await toggle.getAttribute('aria-expanded'), 'true')

  const ahead = ['p1', 'p2', 'p3', 'p4', 'p5', 'p6'].map((id) => row(id, null, { attention: invited }))
  await page.evaluate((pages) => window.setQueue({ pages, maintainer: true, loading: false }), { active: { ...empty, requests: [...ahead, ...requests.slice(0, 3)] }, unclaimed: empty, set_aside: empty, done: empty })
  await page.locator('[data-request-id="p6"]').waitFor()
  await baseRow.hover()
  await baseRow.getByRole('button', { name: 'Collapse 2 stacked requests' }).click()
  await page.locator('[data-request-id="first"]').waitFor({ state: 'detached' })
  await page.keyboard.press('[')
  await page.locator('.request-workspace-sidebar[data-state="closed"]:not([data-closing])').waitFor()
  assert.equal(await baseRow.isVisible(), true)
  assert.equal(await baseRow.locator('.request-workspace-row-avatar').evaluate((node) => getComputedStyle(node).outlineStyle), 'solid')
  assert.equal(await page.locator('.request-workspace-rail-more').textContent(), '+2')
  await baseRow.locator('.request-workspace-row-avatar').hover()
  const stackHint = page.locator('.request-workspace-rail-hint')
  assert.deepEqual(await stackHintItems(stackHint), [
    ['Stacked base', 'review', 'Your request', 'Sep 21', 0, true],
    ['Stacked first', 'review', 'Your request', null, 1, false],
    ['Stacked ready', 'merge', 'Ready to merge', null, 2, false],
  ])
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.stack-rail-hint.png` })
  await page.mouse.move(1200, 800)
  await stackHint.waitFor({ state: 'detached' })
  await baseRow.hover()

  await page.keyboard.press('[')
  await baseRow.getByRole('button', { name: 'Expand 2 stacked requests' }).click()
  await page.clock.install({ time: new Date('2026-10-20T12:00:00Z') })
  await page.clock.pauseAt(new Date('2026-10-20T12:00:00Z'))
  const nestedToggle = page.locator('[data-request-id="first"] .request-workspace-stack-toggle')
  await toggle.evaluate((button) => button.click())
  await nestedToggle.evaluate((button) => button.click())
  await toggle.evaluate((button) => button.click())
  await page.clock.runFor(100)
  assert.deepEqual(await ids(), ['p1', 'p2', 'p3', 'p4', 'p5', 'p6', 'base', 'first'])
  assert.deepEqual(errors, [])
})
