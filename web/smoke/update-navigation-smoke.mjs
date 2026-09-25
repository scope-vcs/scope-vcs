import assert from 'node:assert/strict'
import { assertDocumentPreserved, markDocument, waitForClientHydration } from './browser-smoke.mjs'
import { serverFunctionName } from './server-functions-smoke.mjs'

// An update page opens no diff until a file is chosen, and choosing or closing
// one stays a client navigation that reuses the loaded entry.
export async function assertUpdateFileSelectionKeepsDocument(page) {
  await page.getByText('Select a changed file', { exact: true }).waitFor()
  assert.equal(new URL(page.url()).searchParams.has('path'), false)
  const fileNavigator = page.getByLabel('Update file navigator')
  await waitForClientHydration(fileNavigator)
  await page.waitForFunction(
    () => globalThis.__TSR_ROUTER__.state.status === 'idle',
  )
  const documentSentinel = 'scope-update-file-selection'
  await markDocument(page, documentSentinel)
  const serverFunctions = []
  const recordServerFunction = (request) => {
    if (request.url().includes('/_serverFn/')) {
      const name = serverFunctionName(request)
      // Live repository refresh can run independently of file selection.
      if (name.startsWith('loadHistoryEntry_')) serverFunctions.push(name)
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

  await page.getByRole('button', { name: 'Close diff viewer' }).click()
  await page.waitForURL((url) => !url.searchParams.has('path'))
  await page.getByText('Select a changed file', { exact: true }).waitFor()
  await assertDocumentPreserved(page, documentSentinel)
}
