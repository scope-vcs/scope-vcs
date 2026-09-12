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
  'loadAnalyticsIdentity_createServerFn_handler',
  'loadAttachmentLimits_createServerFn_handler',
])

export function captureRequestShell(page) {
  return captureNodes(page, ['h1', 'nav[aria-label="Request views"]'])
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

export async function assertUpdateSelectionReloadsSelectedPayload(page) {
  await page.locator('[data-slot="pending-surface"]').waitFor({ state: 'detached' })
  await page.locator('summary').filter({ hasText: /^commits ·/ }).click()
  const updates = page.getByRole('button', {
    name: /, commit .+, \d+ files?$/,
  })
  assert(await updates.count() > 1, 'expected more than one request update')
  const target = updates.nth(1)
  const commit = await target.getAttribute('title')
  const targetLabel = await target.getAttribute('aria-label')
  assert(commit)
  assert(targetLabel)
  const title = targetLabel.split(', commit ', 1)[0]
  const serverFunctions = []
  const recordServerFunction = (request) => {
    if (request.url().includes('/_serverFn/')) {
      const name = serverFunctionName(request)
      if (!backgroundServerFunctions.has(name)) serverFunctions.push(name)
    }
  }
  page.on('request', recordServerFunction)
  try {
    await target.click()
    await page.waitForURL((url) => (
      url.searchParams.get('commit') === commit &&
      !url.searchParams.has('path')
    ))
    await page.getByRole('heading', { level: 3, name: title }).waitFor()
  } finally {
    page.off('request', recordServerFunction)
  }
  assert.deepEqual(serverFunctions, ['loadChangesPage_createServerFn_handler'])
}
