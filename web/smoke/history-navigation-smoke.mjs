import assert from 'node:assert/strict'

export async function assertHistoryFirstFileStaysInRoute(page) {
  const defaultDiff = page.getByLabel('README.html diff', { exact: true })
  await defaultDiff.waitFor()
  await defaultDiff.locator('[data-slot="pending-surface"]').waitFor({ state: 'detached' })
  const fileNavigator = page.getByLabel('Update file navigator')
  await fileNavigator.waitFor()
  await page.waitForFunction(
    (element) => Object.keys(element).some((key) => key.startsWith('__reactProps$')),
    await fileNavigator.elementHandle(),
  )
  await page.waitForFunction(
    () => globalThis.__TSR_ROUTER__.state.status === 'idle',
  )
  const documentSentinel = 'scope-history-file-selection'
  await page.evaluate((sentinel) => {
    window.__scopeHistoryDocument = sentinel
  }, documentSentinel)
  const serverFunctions = []
  const recordServerFunction = (request) => {
    if (request.url().includes('/_serverFn/')) {
      const name = serverFunctionExport(request)
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
  assert.equal(
    await page.evaluate(() => window.__scopeHistoryDocument),
    documentSentinel,
  )
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
}

function serverFunctionExport(request) {
  const encodedId = new URL(request.url()).pathname.split('/').at(-1)
  assert(encodedId, 'server function request is missing its encoded id')
  return JSON.parse(Buffer.from(encodedId, 'base64url')).export
}

export async function assertHistoryFeedNavigation(page) {
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
  assert.equal(await page.evaluate(() => window.__scopeHistoryDocument), 'scope-history-file-selection')
}
