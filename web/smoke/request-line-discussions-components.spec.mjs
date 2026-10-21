import assert from 'node:assert/strict'
import test from 'node:test'
import { Worker } from 'node:worker_threads'
import { fixtureClerk, openFixture } from './support/fixture-server.mjs'

const oldText = ['alpha', 'bravo', 'charlie', 'delta', 'echo', 'foxtrot', 'golf', 'hotel', ''].join('\n')
const newText = ['alpha', 'bravo', 'CHARLIE', 'delta', 'echo', 'foxtrot', 'golf', 'hotel', 'india', ''].join('\n')

async function renderDiff() {
  const worker = new Worker(new URL('../src/features/review/review-file-diff-render-worker.ts', import.meta.url))
  try {
    return await new Promise((resolve, reject) => {
      worker.on('error', reject)
      worker.on('message', (message) => {
        if (message.kind === 'ready') {
          worker.postMessage({ budget: { maxHunks: 64, maxOutputBytes: 262_144, maxRenderedLines: 800 }, newText, oldText, path: '/src/lib.ts' })
        } else if (message.kind === 'html') resolve(message.html)
        else reject(new Error(`diff rendered as ${message.kind}`))
      })
    })
  } finally {
    await worker.terminate()
  }
}

const screenshot = process.env.SCOPE_COMPONENT_SCREENSHOT

async function assertFits(page, name, regions) {
  for (const [label, width] of [['desktop', 1280], ['mobile', 390], ['narrow', 320]]) {
    await page.setViewportSize({ width, height: 900 })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, `${name} overflows at ${width}`)
    for (const region of regions) {
      const box = await region.boundingBox()
      assert.ok(box && box.x >= 0 && box.x + box.width <= width + 0.5, `${name} clips at ${width}`)
    }
    const clipped = await page.evaluate(() => [...document.querySelectorAll('ul[aria-label="File discussions"] a, ul[aria-label^="Discussions on"] a')]
      .flatMap((link) => [...link.children].filter((child) => child.getBoundingClientRect().right > link.getBoundingClientRect().right + 0.5)))
    assert.equal(clipped.length, 0, `${name} clips a discussion row at ${width}`)
    if (screenshot) await page.screenshot({ path: `${screenshot}.${name}.${label}.png`, fullPage: true })
  }
  await page.setViewportSize({ width: 1280, height: 900 })
}

test('line discussions open from either side, post under their lines, and keep carried, outdated and fallback references', async (t) => {
  const html = await renderDiff()
  const { base, errors, page } = await openFixture(t, 'request-line-discussions', { aliases: [fixtureClerk], tailwind: true, timeout: 30_000 })
  await page.addInitScript((value) => { window.diffHtml = value }, html)
  await page.goto(new URL('/adam/demo/requests/request/changes', base).href)

  const diff = page.getByRole('complementary', { name: 'src/lib.ts diff' })
  const fileNotes = diff.getByRole('list', { name: 'File discussions' })
  await fileNotes.waitFor()
  assert.deepEqual(await fileNotes.getByRole('listitem').allInnerTexts(), [
    'Lines outside the shown hunks\nline 40 (new) not shown\nOpen',
    'Outdated naming question\nOutdated\nOpen',
  ])
  const carried = diff.getByRole('list', { name: 'Discussions on new line 6' })
  assert.equal(await carried.getByRole('link', { name: /Carried forward review/ }).innerText(), 'Carried forward review\nlines 5-6 (new)\nOpen')
  const anchors = page.getByRole('list', { name: 'Discussion page anchors' })
  assert.deepEqual(await anchors.getByRole('link').allInnerTexts(), ['rev 1 · L4-5 → L5-6', 'rev 1 · L3 · outdated'])
  assert.deepEqual(await anchors.getByRole('link').evaluateAll((links) => links.map((link) => link.getAttribute('aria-label'))), [
    'View revision 1 changes for src/lib.ts lines 4-5 (new), now lines 5-6 (new) on the latest revision',
    'View revision 1 changes for src/lib.ts line 3 (new), outdated on the latest revision',
  ])
  const related = page.locator('[aria-label="Related discussions"]')
  assert.equal(await related.getByRole('link', { name: /Overall revision review/ }).count(), 1)
  assert.match(await related.getByRole('link', { name: /Outdated naming question/ }).innerText(), /Outdated · Open/)
  await assertFits(page, 'references', [fileNotes, carried])

  const gutter = (type, line) => diff.locator(`[data-gutter] [data-line-type="${type}"][data-column-number="${line}"]`)
  await gutter('change-addition', 3).hover()
  assert.equal(await gutter('change-addition', 3).evaluate((cell) => getComputedStyle(cell).cursor), 'pointer')
  await gutter('change-addition', 3).click()
  await diff.getByRole('textbox', { name: 'Start a discussion on line 3 (new)' }).waitFor()
  await gutter('change-deletion', 3).click()
  await diff.getByRole('textbox', { name: 'Start a discussion on line 3 (old)' }).waitFor()
  assert.equal(await diff.getByRole('textbox', { name: 'Start a discussion on line 3 (new)' }).count(), 0)

  await gutter('context', 7).click()
  await gutter('context', 8).click({ modifiers: ['Shift'] })
  const composer = diff.getByRole('textbox', { name: 'Start a discussion on lines 7-8 (new)' })
  await composer.waitFor()
  assert.equal(await diff.locator('[data-selected-line]').count(), 4)
  await assertFits(page, 'composer', [composer, diff.getByRole('button', { name: 'Start discussion' })])

  await composer.fill('Check the tail lines')
  await gutter('change-addition', 9).click({ modifiers: ['Shift'] })
  const extended = diff.getByRole('textbox', { name: 'Start a discussion on lines 7-9 (new)' })
  await extended.waitFor()
  assert.equal(await extended.inputValue(), 'Check the tail lines')
  assert.equal(await diff.getByRole('button', { name: 'Start discussion' }).isEnabled(), true)
  await diff.getByRole('button', { name: 'Start discussion' }).click()
  const posted = diff.getByRole('list', { name: 'Discussions on new line 9' })
  await posted.getByRole('link', { name: /Check the tail lines/ }).waitFor()
  assert.equal(await extended.count(), 0)
  const [created] = await page.evaluate(() => window.createCalls)
  assert.deepEqual(created.anchor, {
    commit_oid: null, lines: { side: 'new', start_line: 7, end_line: 9 }, path: '/src/lib.ts', revision_id: 'revision-2',
  })
  assert.equal(await posted.innerText(), 'Check the tail lines\nlines 7-9 (new)\nOpen')
  await assertFits(page, 'posted', [posted, carried, fileNotes])
  assert.deepEqual(errors, [])
})
