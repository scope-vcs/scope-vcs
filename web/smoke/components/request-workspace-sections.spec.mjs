import assert from 'node:assert/strict'
import test from 'node:test'
import { fixtureClerk, fixtureRepositorySearch, openFixture } from '../support/fixture-server.mjs'

function row(id, group, nextStep, status) {
  return {
    request: {
      id,
      title: `Request ${id}`,
      stack_parent: null,
      stack_depth: 1,
      mergeability: { status, current_main_oid: null, request_head_oid: 'head', reason: null },
      diff_stat: { additions: 12, deletions: 3, changed_files: 2 },
    },
    author: { handle: 'adam' },
    attention: { group, stack_group: group, reason: 'authored', can_set_aside: true },
    attention_at_unix: 1_790_000_000,
    next_step: nextStep,
  }
}

test('opening a set-aside or done request leaves it in its group until the collapsed rail needs it', async (t) => {
  const { base, errors, page } = await openFixture(t, 'request-workspace', { aliases: [fixtureClerk, fixtureRepositorySearch], tailwind: true })
  await page.setViewportSize({ width: 1280, height: 1000 })
  await page.goto(new URL('/adam/demo/requests/selected', base).href)
  await page.locator('[data-request-id]').first().waitFor()
  const queuePage = (requests) => ({ requests, next_cursor: null, next_attention_at_unix: null })
  await page.evaluate((pages) => window.setQueue({ pages, maintainer: true, loading: false }), {
    active: queuePage([row('ready', 'needs_you', 'merge', 'Ready')]),
    unclaimed: queuePage([]),
    set_aside: queuePage([row('snoozed', 'set_aside', 'merge', 'Ready')]),
    done: queuePage([row('merged', 'done', 'done', 'Merged')]),
  })
  const sidebar = page.locator('.request-workspace-sidebar')
  const needsYou = () => sidebar.locator('.request-workspace-needs-you [data-request-id]').evaluateAll((rows) => rows.map((row) => row.dataset.requestId))
  const group = (id) => sidebar.locator(`[data-request-id="${id}"]`).evaluate((row) => row.closest('.request-workspace-disclosures, .request-workspace-current, .request-workspace-needs-you').className)
  for (const [label, id] of [['Set aside', 'snoozed'], ['Done', 'merged']]) {
    await sidebar.getByRole('button', { name: new RegExp(label) }).click()
    const link = sidebar.locator(`[data-request-id="${id}"] .request-workspace-row-link`)
    const before = await link.boundingBox()
    await link.click()
    await page.waitForURL(new RegExp(`/${id}$`))
    assert.equal(await link.getAttribute('aria-current'), 'page')
    assert.deepEqual(await needsYou(), ['ready'], `opening ${id} keeps it out of Needs you`)
    assert.equal(await group(id), 'request-workspace-disclosures', `${id} stays in ${label}`)
    assert.deepEqual(await link.boundingBox(), before, `${id} stays where it was clicked`)
  }
  await page.keyboard.press('[')
  await page.locator('.request-workspace-sidebar[data-state="closed"]:not([data-closing])').waitFor()
  assert.equal(await group('merged'), 'request-workspace-current', 'the collapsed rail still shows the open request')
  assert.deepEqual(errors, [])
})
