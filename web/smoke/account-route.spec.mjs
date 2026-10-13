import assert from 'node:assert/strict'
import test from 'node:test'
import { fixtureClerk, fixturePath, openFixture } from './support/fixture-server.mjs'

const ui = fixturePath('account-route', 'ui.tsx')
const mockedUi = [
  'components/application-topbar', 'components/app-shell', 'components/copyable-code-block',
  'components/page-header', 'components/page-error-alert', 'components/section-rows',
  'components/ui/button', 'components/timestamp',
  'features/account/account-page-header', 'features/account/account-page-pending',
  'features/account/account-sections', 'features/account/cli-session-list',
  'features/account/delete-account-section',
]

test('account route shows revalidated CLI sessions while mounted', async (t) => {
  const { base, page } = await openFixture(t, 'account-route', {
    timeout: 5000,
    aliases: [
      ...mockedUi.map((name) => ({ find: `@/${name}`, replacement: ui })),
      { find: '@tanstack/react-router', replacement: fixturePath('account-route', 'router.ts') },
      { find: '@tanstack/react-start', replacement: fixturePath('account-route', 'server.ts') },
      { find: '@clerk/tanstack-react-start/server', replacement: fixtureClerk.replacement },
      fixtureClerk,
    ],
  })
  await page.goto(base)
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
