import assert from 'node:assert/strict'
import test from 'node:test'
import { fixtureClerk, fixtureRepositorySearch, openFixture } from '../support/fixture-server.mjs'

const parent = (id) => ({ kind: 'available', request: { id, name: id, title: id, state: 'Open', stack_status: 'main', stack_depth: 1, head_oid: 'head', current_revision_id: 'revision' } })

function row(id, nextStep, status, { parentId = null, diff = true, reason = 'authored' } = {}) {
  return {
    request: {
      id,
      title: `Request ${id}`,
      stack_parent: parentId && parent(parentId),
      stack_depth: parentId ? 2 : 1,
      mergeability: { status, current_main_oid: null, request_head_oid: 'head', reason: null },
      diff_stat: diff ? { additions: 12, deletions: 3, changed_files: 2 } : null,
    },
    author: { handle: 'adam' },
    attention: { group: 'needs_you', stack_group: 'needs_you', reason, can_set_aside: true },
    attention_at_unix: 1_790_000_000,
    next_step: nextStep,
  }
}

test('the collapsed rail spaces the toggle, count, icons, stack outlines, rule and overflow count evenly', async (t) => {
  const { base, errors, page } = await openFixture(t, 'request-workspace', { aliases: [fixtureClerk, fixtureRepositorySearch], tailwind: true })
  await page.setViewportSize({ width: 1280, height: 1000 })
  await page.goto(new URL('/adam/demo/requests/selected', base).href)
  await page.locator('[data-request-id]').first().waitFor()
  const requests = [
    row('draft', 'submit', 'Draft', { diff: false }),
    row('fetch', 'fix_checks', 'ChecksFailed'),
    row('list', 'wait', 'WaitingForParent', { parentId: 'fetch', diff: false }),
    row('open-parent', 'fix_checks', 'ChecksFailed'),
    row('open-child', 'wait', 'WaitingForParent', { parentId: 'open-parent' }),
    row('verify', 'merge', 'Ready'),
    row('delete', 'wait', 'WaitingForParent', { parentId: 'verify' }),
    row('plain', 'merge', 'Ready'),
    ...['a', 'b', 'c'].map((id) => row(id, 'fix_checks', 'ChecksFailed')),
    row('selected', 'review', 'Ready', { reason: 'claimed' }),
  ]
  const empty = { requests: [], next_cursor: null, next_attention_at_unix: null }
  await page.evaluate((pages) => window.setQueue({ pages, maintainer: true, loading: false }), { active: { ...empty, requests }, unclaimed: empty, set_aside: empty, done: empty })
  await page.locator('[data-request-id="c"]').waitFor()
  for (const id of ['fetch', 'verify']) await page.locator(`[data-request-id="${id}"] .request-workspace-stack-pill`).click()
  await page.keyboard.press('[')
  await page.locator('.request-workspace-sidebar[data-state="closed"]:not([data-closing])').waitFor()
  await page.mouse.move(900, 900)
  await page.waitForFunction(() => document.getAnimations().every((animation) => !(animation instanceof CSSTransition)))
  const blocks = await page.evaluate(() => {
    const sidebar = document.querySelector('.request-workspace-sidebar')
    const span = (element, grow = 0) => {
      const { top, bottom } = element.getBoundingClientRect()
      return { top: top - grow, bottom: bottom + grow }
    }
    const result = [
      ['toggle', span(sidebar.querySelector('.request-workspace-sidebar-toggle svg'))],
      ['count', span(sidebar.querySelector('.request-workspace-needs-you > h2 > span:last-child'))],
    ]
    let outline = null
    for (const node of sidebar.querySelectorAll('.request-workspace-row')) {
      if (getComputedStyle(node).display === 'none') continue
      const edge = node.dataset.stackEdge
      const box = node.getBoundingClientRect()
      const before = getComputedStyle(node, '::before')
      if (edge === 'start' || edge === 'single') outline = { top: box.top + parseFloat(before.top) }
      if (edge === 'end' || edge === 'single') {
        result.push([`stack ending ${node.dataset.requestId}`, { ...outline, bottom: box.bottom - parseFloat(before.bottom) }])
        outline = null
      }
      if (edge) continue
      const ring = node.classList.contains('request-workspace-row--selected') ? 4.5 : 0
      result.push([node.dataset.requestId, span(node.querySelector('.request-workspace-row-mark'), ring)])
    }
    for (const element of sidebar.querySelectorAll('.request-workspace-rail-rule, .request-workspace-rail-more')) {
      if (getComputedStyle(element).display !== 'none') result.push([element.className, span(element)])
    }
    return result
  })
  assert.deepEqual(blocks.map(([name]) => name), [
    'toggle', 'count', 'draft', 'stack ending fetch', 'stack ending open-child', 'stack ending verify', 'plain',
    'selected', 'request-workspace-rail-rule', 'request-workspace-rail-more',
  ])
  const gaps = blocks.slice(1).map(([name, { top }], index) => [name, Math.round(top - blocks[index][1].bottom)])
  for (const [name, gap] of gaps) assert.equal(gap, 20, `${name} sits 20px below the element above it, not ${gap}px`)
  assert.deepEqual(errors, [])
})
