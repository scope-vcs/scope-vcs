import {
  assertDocumentPreserved,
  assertNodesPreserved,
  captureRepositoryChrome,
  markDocument,
  requestRepoPath,
} from './browser-smoke.mjs'

export async function assertRepositoryMarkdownUsesClientNavigation(page) {
  await page.getByRole('button', { name: 'README.md', exact: true }).click()
  await page.getByRole('heading', { level: 1, name: 'Update Demo' }).waitFor()
  const chrome = await captureRepositoryChrome(page)
  const documentSentinel = 'scope-repository-markdown-navigation'
  await markDocument(page, documentSentinel)

  await page.getByRole('link', { name: 'Read the release guide' }).click()
  await page.waitForURL((url) => (
    url.pathname === requestRepoPath &&
    url.searchParams.get('file') === 'docs/release.md'
  ))
  await page.getByRole('heading', { level: 1, name: 'Release flow' }).waitFor()
  await assertDocumentPreserved(page, documentSentinel)
  await assertNodesPreserved(page, chrome)
}
