import assert from 'node:assert/strict'
import test from 'node:test'
import { fixturePath, openFixture } from '../support/fixture-server.mjs'

test('visibility timeline shows the focused update under the chart without covering it', async (t) => {
  const { base, errors, page } = await openFixture(t, 'visibility-timeline', {
    tailwind: true,
    aliases: [
      { find: '@tanstack/react-router', replacement: fixturePath('visibility-timeline', 'router.tsx') },
      { find: '@/features/history/history-feed', replacement: fixturePath('visibility-timeline', 'history-feed.ts') },
    ],
  })
  await page.goto(base)
  const chart = page.getByRole('img', { name: '6 most recent visibility changes, oldest to newest' })
  const detail = page.locator('main')
  await chart.waitFor()
  const shownCounts = () => chart.locator('text[fill="var(--foreground)"]').allTextContents()
  const pointAboveColumn = async (count) => {
    const label = await chart.locator('text').filter({ hasText: new RegExp(`^${count}$`) }).boundingBox()
    const plot = await chart.boundingBox()
    return [label.x + label.width / 2, plot.y + 4]
  }

  for (const line of ['Latest', 'Publish examples and templates', 'Visibility · maya', '282 entered the Public view']) {
    await detail.getByText(line, { exact: true }).waitFor()
  }
  assert.deepEqual(await shownCounts(), ['282'])
  assert.equal(await page.getByRole('tooltip').count(), 0)

  await page.mouse.move(...await pointAboveColumn(31))
  await detail.getByText('Publish API reference', { exact: true }).waitFor()
  assert.equal(await detail.getByText('Latest', { exact: true }).count(), 0)
  assert.deepEqual(await shownCounts(), ['31'])

  await page.mouse.move(...await pointAboveColumn(92))
  await detail.getByText('210 files, 92 entered the Public view, 18 left the Public view', { exact: true }).waitFor()
  assert.deepEqual(await shownCounts(), ['92', '18'])
  assert.deepEqual(await chart.locator('path[opacity]').evaluateAll((paths) => paths.map((path) => path.dataset.tsKey.split(':').slice(-2).join(':'))), [
    'entry-0:entered', 'entry-3:left', 'entry-9:entered', 'entry-17:entered', 'entry-38:entered',
  ])

  await page.mouse.move(...await pointAboveColumn(22))
  await detail.getByText('22 left the Public view', { exact: true }).waitFor()
  const exitCount = await chart.locator('text').filter({ hasText: /^22$/ }).boundingBox()
  const exitDate = await chart.getByText('Aug 20', { exact: true }).boundingBox()
  assert(exitCount.y + exitCount.height <= exitDate.y, 'the count under an exit bar stays above its date')

  await page.mouse.move(0, 0)
  await detail.getByText('Latest', { exact: true }).waitFor()
  assert.deepEqual(await shownCounts(), ['282'])

  await chart.focus()
  await page.keyboard.press('ArrowRight')
  await detail.getByText('Hide internal docs', { exact: true }).waitFor()
  await page.keyboard.press('Enter')
  await page.mouse.click(...await pointAboveColumn(126))
  assert.deepEqual(await page.evaluate(() => window.navigations.map(({ params }) => params.entryId)), ['source-3', 'source-17'])
  assert.deepEqual(errors, [])
})
