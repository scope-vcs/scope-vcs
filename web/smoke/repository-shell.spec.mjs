import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  assertCurrentRepoSection,
  assertPageHeading,
  assertPassiveSkeleton,
  baseUrl,
  repoPath,
  withBlankPage,
  within,
  withPage,
} from './browser-smoke.mjs'

test('repository shell renders before the initial file is ready', async () => {
  for (const scenario of [
    {
      content: (page) => page.locator('iframe[title="README.html preview"]'),
      path: repoPath,
      requestPath: 'README.html',
    },
    {
      content: (page) => page.locator('pre code').filter({
        hasText: 'export function greet',
      }),
      path: `${repoPath}?file=src%2Fapp.ts`,
      requestPath: 'src/app.ts',
    },
  ]) {
    await assertShellBeforeFileReady(scenario)
  }
})

test('unknown repository-shaped paths return not found', async () => {
  await withBlankPage(async (page) => {
    for (const path of [
      '/wp-admin/install.php',
      '/definitely-no-such-owner/definitely-no-such-repo',
    ]) {
      const response = await page.goto(new URL(path, `${baseUrl}/`).toString(), {
        timeout: 30_000,
        waitUntil: 'domcontentloaded',
      })
      assert(response, `navigation to ${path} did not produce a response`)
      assert.equal(response.status(), 404)
      await page
        .getByRole('heading', { level: 1, name: 'Nothing lives at this address.' })
        .waitFor()
    }
  })
})

test('public direct Runs access is explicit and exposes no operations', async () => {
  await withPage(`${repoPath}/runs`, async (page) => {
    await assertPageHeading(page, 'Runs')
    await page.getByText(
      'Sign in as the owner or a repository member to view runs.',
      { exact: true },
    ).waitFor()
    assert.equal(
      await page
        .getByRole('navigation', { name: 'Primary' })
        .getByRole('link', { name: 'Runs', exact: true })
        .count(),
      0,
    )
    assert.equal(
      await page.getByRole('heading', { name: 'Recent runs' }).count(),
      0,
    )
    assert.equal(await page.getByRole('heading', { name: 'Runners' }).count(), 0)
  })
})

async function assertShellBeforeFileReady({ content, path, requestPath }) {
  let fileRequests = 0
  let releaseFileRequest = () => undefined
  let markFileRequestStarted = () => undefined
  const fileRequestStarted = new Promise((resolve) => {
    markFileRequestStarted = resolve
  })
  const fileRequestReleased = new Promise((resolve) => {
    releaseFileRequest = resolve
  })

  try {
    await withPage(
      path,
      async (page) => {
        await within(
          fileRequestStarted,
          10_000,
          `${requestPath} request did not start`,
        )
        await assertCurrentRepoSection(page, 'Code')
        await assertPageHeading(page, 'Code')
        await page.getByLabel('Repository file navigator').waitFor()
        await page
          .locator('#repository-code-files-panel [aria-busy="true"]')
          .waitFor()
        await assertPassiveSkeleton(page, '#repository-code-files-panel')
        assert.equal(await content(page).count(), 0)

        releaseFileRequest()
        await content(page).waitFor()
        assert.equal(fileRequests, 1)
      },
      {
        prepare: (page) => page.route('**/_serverFn/**', async (route) => {
          const request = route.request()
          if (
            request.method() !== 'GET' ||
            !decodeURIComponent(request.url()).includes(requestPath)
          ) {
            await route.continue()
            return
          }
          fileRequests += 1
          markFileRequestStarted()
          await fileRequestReleased
          await route.continue()
        }),
      },
    )
  } finally {
    releaseFileRequest()
  }
}
