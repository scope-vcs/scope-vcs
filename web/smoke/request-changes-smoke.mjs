import assert from 'node:assert/strict'
import {
  assertDocumentPreserved,
  assertNodesPreserved,
  captureNodes,
  markDocument,
  waitForClientHydration,
} from './browser-smoke.mjs'
import { serverFunctionName } from './server-functions-smoke.mjs'

const backgroundServerFunctions = new Set([
  'listRequestAttachments_createServerFn_handler',
  'loadAccountSession_createServerFn_handler',
  'loadAttachmentLimits_createServerFn_handler',
])

// The discussion and the changes screen replace each other inside the
// requests workspace, which stays mounted.
export function captureRequestShell(page) {
  return captureNodes(page, ['[aria-label="Requests workspace"]'])
}

export function changesBackLink(page) {
  return page.getByRole('navigation', { name: 'Request changes navigation' })
    .getByRole('link', { name: 'Discussion', exact: true })
}

export async function assertRequestCrossLinksStayInDocument(page) {
  await page.getByRole('heading', { level: 1, name: 'Add bounded retry timing' }).waitFor()
  const shell = await captureRequestShell(page)
  const documentSentinel = 'scope-request-cross-navigation'
  await markDocument(page, documentSentinel)

  const anchoredThread = page.locator(
    '#discussion-discussion_demo_revision_jitter',
  )
  const revisionLink = anchoredThread.getByRole('link', { name: /View revision/ })
  await waitForClientHydration(revisionLink)
  await revisionLink.click()
  await page.waitForURL((url) => (
    url.pathname.endsWith('/requests/req_demo_ready/changes') &&
    url.searchParams.get('revision') === 'event_req_demo_ready_revision_2'
  ))
  await assertDocumentPreserved(page, documentSentinel)
  await assertNodesPreserved(page, shell)
  assert.equal(await page.getByRole('textbox').count(), 0)

  const discussionLink = page.getByRole('link', {
    name: /The bounded jitter looks right/,
  })
  await waitForClientHydration(discussionLink)
  await discussionLink.click()
  await page.waitForURL((url) => (
    url.pathname.endsWith('/requests/req_demo_ready') &&
    url.searchParams.get('discussion') === 'discussion_demo_revision_jitter' &&
    url.hash === '#discussion-discussion_demo_revision_jitter'
  ))
  await page.locator('.request-discussion-thread').first().waitFor()
  await assertDocumentPreserved(page, documentSentinel)
  await assertNodesPreserved(page, shell)
}

export async function assertFileSelectionSkipsRevisionReload(page, fileName, path) {
  await page.locator('[data-slot="pending-surface"]').waitFor({ state: 'detached' })
  const fileNavigator = page.getByLabel('Commit file navigator')
  await waitForClientHydration(fileNavigator)
  const serverFunctions = []
  const recordServerFunction = (request) => {
    if (request.url().includes('/_serverFn/')) {
      const name = serverFunctionName(request)
      if (!backgroundServerFunctions.has(name)) serverFunctions.push(name)
    }
  }
  page.on('request', recordServerFunction)
  try {
    for (const folder of path.split('/').filter(Boolean).slice(0, -1)) {
      const expandFolder = fileNavigator.getByRole('button', {
        exact: true,
        name: `Expand ${folder}`,
      })
      if (await expandFolder.count()) await expandFolder.click()
    }
    await fileNavigator.getByRole('button', { name: fileName }).click()
    await page.waitForURL((url) => url.searchParams.get('path') === path)
    const diff = page.getByLabel(`${path.replace(/^\/+/, '')} diff`, { exact: true })
    await diff.waitFor()
    await diff.locator('[data-slot="pending-surface"]').waitFor({ state: 'detached' })
  } finally {
    page.off('request', recordServerFunction)
  }
  assert.deepEqual(serverFunctions, ['loadRevisionDiff_createServerFn_handler'])
}

export async function assertRevisionStepReloadsSelectedPayload(page) {
  await page.locator('[data-slot="pending-surface"]').waitFor({ state: 'detached' })
  const selectedRevision = await page.getByRole('heading', { level: 1, name: /^Revision \d+$/ }).textContent()
  const older = page.getByRole('link', { name: 'Older' })
  await waitForClientHydration(older)
  const olderRevision = await older.getAttribute('title')
  assert(olderRevision && olderRevision !== selectedRevision, 'expected an older revision')
  const olderRevisionId = new URL(await older.getAttribute('href'), page.url()).searchParams.get('revision')
  const serverFunctions = []
  const recordServerFunction = (request) => {
    if (request.url().includes('/_serverFn/')) {
      const name = serverFunctionName(request)
      if (!backgroundServerFunctions.has(name)) serverFunctions.push(name)
    }
  }
  page.on('request', recordServerFunction)
  try {
    await older.click()
    await page.waitForURL((url) => (
      url.searchParams.get('revision') === olderRevisionId &&
      !url.searchParams.has('path')
    ))
    await page.getByRole('heading', { level: 1, name: olderRevision }).waitFor()
    await page.locator('[data-slot="pending-surface"]').waitFor({ state: 'detached' })
  } finally {
    page.off('request', recordServerFunction)
  }
  assert.deepEqual(serverFunctions, ['loadRevisions_createServerFn_handler', 'loadDiscussions_createServerFn_handler'])

  const selectedUrl = page.url()
  await changesBackLink(page).click()
  await page.waitForURL((url) => !url.pathname.endsWith('/changes'))
  await page.locator('.request-discussion-thread').first().waitFor()
  const reopenedLoads = []
  const recordReopenedLoad = (request) => {
    if (!request.url().includes('/_serverFn/')) return
    const name = serverFunctionName(request)
    if (name === 'loadRevisions_createServerFn_handler' || name === 'loadDiscussions_createServerFn_handler') {
      reopenedLoads.push(name)
    }
  }
  page.on('request', recordReopenedLoad)
  try {
    await page.goBack()
    await page.waitForURL(selectedUrl)
    await page.getByRole('heading', { level: 1, name: olderRevision }).waitFor()
    await page.locator('[data-slot="pending-surface"]').waitFor({ state: 'detached' })
    assert.deepEqual(reopenedLoads, [])
  } finally {
    page.off('request', recordReopenedLoad)
  }
}
