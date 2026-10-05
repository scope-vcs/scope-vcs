import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'

const require = createRequire(import.meta.url)
const fixture = (path) => fileURLToPath(new URL(`./fixtures/account-route/${path}`, import.meta.url))
const ui = fixture('ui.tsx')
const mockedUi = [
  'components/application-topbar', 'components/app-shell', 'components/copyable-code-block',
  'components/page-header', 'components/page-error-alert', 'components/section-rows',
  'components/ui/button', 'components/timestamp',
  'features/account/account-page-header', 'features/account/account-page-pending',
  'features/account/account-sections', 'features/account/cli-session-list',
  'features/account/delete-account-section',
]

test('account route shows revalidated CLI sessions while mounted', async (t) => {
  const cacheDir = await mkdtemp(join(tmpdir(), 'scope-account-route-'))
  t.after(() => rm(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    cacheDir, configFile: false, root: fixture('.'),
    server: { host: '127.0.0.1', port: 0, fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] } },
    resolve: { alias: [
      ...mockedUi.map((name) => ({ find: `@/${name}`, replacement: ui })),
      { find: '@tanstack/react-router', replacement: fixture('router.ts') },
      { find: '@tanstack/react-start', replacement: fixture('server.ts') },
      { find: '@clerk/tanstack-react-start/server', replacement: fixture('clerk.tsx') },
      { find: '@clerk/tanstack-react-start', replacement: fixture('clerk.tsx') },
      { find: '@', replacement: fileURLToPath(new URL('../src', import.meta.url)) },
      ...['react/jsx-dev-runtime', 'react/jsx-runtime', 'react-dom/client', 'react'].map(name => ({ find: name, replacement: require.resolve(name) })),
    ] },
    oxc: { jsx: { runtime: 'automatic' } },
  })
  await server.listen()
  t.after(() => server.close())
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  page.setDefaultTimeout(5000)
  await page.goto(server.resolvedUrls.local[0])
  await page.getByText('First CLI').waitFor()
  await page.evaluate(() => window.revalidateAccount())
  await page.getByText('Second CLI').waitFor()
  assert.equal(await page.getByText('First CLI').count(), 0)
  await page.evaluate(() => { window.failRevoke = true })
  await page.getByRole('button', { name: 'Revoke Second CLI' }).click()
  await page.getByText('Revocation denied').waitFor()
  assert.equal(await page.getByRole('button', { name: 'Revoke Second CLI' }).count(), 1)
  await page.evaluate(() => {
    window.failRevoke = false
    window.startDelayedAccountLoad()
  })
  await page.getByRole('button', { name: 'Revoke Second CLI' }).click()
  await page.getByRole('button', { name: 'Revoke Second CLI' }).waitFor({ state: 'detached' })
  await page.evaluate(async () => {
    window.deliverAccountResponse()
    await window.delayedAccountLoad
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))
  })
  assert.equal(await page.getByRole('button', { name: 'Revoke Second CLI' }).count(), 0,
    'a loader response captured before revocation must not restore the revoked session')
  await page.evaluate(() => window.leaveAccount())
  await page.getByText('Other page').waitFor()
  await page.evaluate(() => window.returnAccount())
  await page.getByRole('main').waitFor()
  assert.equal(await page.getByRole('button', { name: 'Revoke Second CLI' }).count(), 0)
  await page.evaluate(() => window.switchAccount())
  await page.getByText('Third CLI').waitFor()
  assert.equal(await page.getByText('First CLI').count(), 0)
})
