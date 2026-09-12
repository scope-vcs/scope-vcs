import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  assertCurrentRepoSection,
  assertPageHeading,
  repoPath,
  requestRepoPath,
  waitForClientHydration,
  withPage,
} from './browser-smoke.mjs'
import { serverFunctionName } from './server-functions-smoke.mjs'

test('public repository requests route is anonymously readable', async () => {
  await withPage(`${repoPath}/requests`, async (page) => {
    await assertCurrentRepoSection(page, 'Requests')
    await assertPageHeading(page, 'Requests')
    assert.equal(await page.getByRole('heading', { level: 2, name: /^your work$/i }).count(), 0)
    await page.getByRole('heading', { level: 2, name: 'open', exact: true }).waitFor()
    await page.locator('summary').filter({ hasText: /^closed/ }).click()
    await page.getByText('No open requests.', { exact: true }).waitFor()
    await page.getByText('No closed requests.', { exact: true }).waitFor()
  })
})

test('request queue search is keyboard accessible and mobile rows do not overflow', async () => {
  await withPage(
    `${requestRepoPath}/requests`,
    async (page) => {
      const readyRow = page.getByRole('link', {
        name: /Add bounded retry timing/,
      })
      await readyRow.waitFor()
      await page.getByRole('link', { name: 'Requests', exact: true }).getByText(/^\d+$/).waitFor()
      await readyRow.focus()
      assert.equal(
        await readyRow.evaluate(
          (element) => element === document.activeElement,
        ),
        true,
      )
      const search = page.getByRole('searchbox', {
        name: 'Search open and closed requests',
      })
      const queueRequests = []
      page.on('request', (request) => {
        if (serverFunctionName(request) === 'loadRequestQueuePage_createServerFn_handler') {
          queueRequests.push(request.url())
        }
      })
      await waitForClientHydration(search)
      await search.fill('missing request title')
      await search.press('Enter')
      // Open and Closed now share the same no-match copy, so scope by section.
      await page
        .getByRole('region', { name: 'Open' })
        .getByText('Nothing matches “missing request title”.', { exact: true })
        .waitFor()
      assert.equal(queueRequests.length, 2)
      const clear = page.getByRole('button', { name: 'Clear' })
      await clear.focus()
      assert.equal(
        await clear.evaluate((element) => element === document.activeElement),
        true,
      )
      assert.equal(
        await page.evaluate(
          () =>
            document.documentElement.scrollWidth <=
            document.documentElement.clientWidth,
        ),
        true,
      )
    },
    { viewport: { height: 844, width: 390 } },
  )
})
