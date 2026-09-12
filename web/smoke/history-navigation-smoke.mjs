import assert from 'node:assert/strict'
import { assertDocumentPreserved, markDocument, waitForClientHydration } from './browser-smoke.mjs'
import { serverFunctionName } from './server-functions-smoke.mjs'

export async function assertHistoryNavigationKeepsDocument(page) {
  const defaultDiff = page.getByLabel('README.html diff', { exact: true })
  await defaultDiff.waitFor()
  await defaultDiff.locator('[data-slot="pending-surface"]').waitFor({ state: 'detached' })
  const fileNavigator = page.getByLabel('Update file navigator')
  await waitForClientHydration(fileNavigator)
  await page.waitForFunction(
    () => globalThis.__TSR_ROUTER__.state.status === 'idle',
  )
  const documentSentinel = 'scope-history-file-selection'
  await markDocument(page, documentSentinel)
  const serverFunctions = []
  const recordServerFunction = (request) => {
    if (request.url().includes('/_serverFn/')) {
      const name = serverFunctionName(request)
      // Live repository refresh can run independently of file selection.
      if (name.startsWith('loadHistoryEntry')) serverFunctions.push(name)
    }
  }
  page.on('request', recordServerFunction)
  try {
    await fileNavigator
      .getByRole('button', { name: 'README.html', exact: true })
      .click()
    await page.waitForURL((url) => (
      url.searchParams.get('path') === '/README.html' &&
      !url.searchParams.has('audience')
    ))
    const diff = page.getByLabel('README.html diff', { exact: true })
    await diff.waitFor()
    await diff.locator('[data-slot="pending-surface"]').waitFor({
      state: 'detached',
    })
  } finally {
    page.off('request', recordServerFunction)
  }
  await assertDocumentPreserved(page, documentSentinel)
  assert.deepEqual(serverFunctions, [])
  await page.waitForFunction((diffLabel) => {
    const diff = document.querySelector(`[aria-label="${diffLabel}"]`)
    const host = diff?.querySelector('diffs-container')
    return (
      host?.shadowRoot &&
      host.shadowRoot.childNodes.length > 0 &&
      host.shadowRoot.textContent?.trim().length > 0
    )
  }, 'README.html diff')

  const activity = page.getByRole('radiogroup', { name: 'History activity' })
  await activity.getByRole('radio', { name: 'All activity', exact: true }).click()
  await page.waitForURL((url) => url.searchParams.get('feed') === 'all')
  await activity.getByRole('radio', { name: 'All activity', exact: true, checked: true }).waitFor()
  await page.getByLabel('History updates', { exact: true }).waitFor()
  assert.equal(await activity.getByRole('radio', { name: 'All activity', exact: true }).getAttribute('aria-checked'), 'true')
  await activity.getByRole('radio', { name: 'Pushes & merges', exact: true }).click()
  await page.waitForURL((url) => url.searchParams.get('feed') === 'updates')
  await activity.getByRole('radio', { name: 'Pushes & merges', exact: true, checked: true }).waitFor()
  await page.getByLabel('History updates', { exact: true }).waitFor()
  await assertDocumentPreserved(page, documentSentinel)
}
