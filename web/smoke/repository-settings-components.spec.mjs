import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import tailwindcss from '@tailwindcss/vite'

const require = createRequire(import.meta.url)
test('repository components retain drafts, previews and pending actions across refreshes and interaction', async (t) => {
  const cacheDir = await mkdtemp(join(tmpdir(), 'scope-vite-components-'))
  t.after(() => rm(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    cacheDir,
    configFile: false,
    root: fileURLToPath(new URL('./fixtures/repository-settings', import.meta.url)),
    plugins: [tailwindcss()],
    server: { host: '127.0.0.1', port: 0, fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] } },
    resolve: { alias: [
      { find: '@', replacement: fileURLToPath(new URL('../src', import.meta.url)) },
      ...['react/jsx-dev-runtime', 'react/jsx-runtime', 'react-dom/client', 'react'].map(name => ({ find: name, replacement: require.resolve(name) })),
    ] },
    oxc: { jsx: { runtime: 'automatic' } },
  })
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ hasTouch: true, viewport: { width: 1280, height: 900 } })
  page.setDefaultTimeout(10000)
  try {
    const errors = []
    page.on('pageerror', error => errors.push(error.message))
    await page.goto(server.resolvedUrls.local[0], { timeout: 30_000, waitUntil: 'domcontentloaded' })
    const initialClose = page.getByTitle('Close Two.html', { exact: true })
    assert.equal(await page.evaluate(() => matchMedia('(any-pointer: coarse)').matches), true)
    assert.equal(await initialClose.evaluate(node => getComputedStyle(node).opacity), '1')
    assert.equal(await initialClose.evaluate(node => getComputedStyle(node).minWidth), '44px')
    await page.getByLabel('Description', { exact: true }).fill('Unsaved draft')
    await page.getByRole('button', { name: 'Remote metadata update' }).click()
    assert.equal(await page.getByLabel('Description', { exact: true }).inputValue(), 'Unsaved draft')
    await page.getByText('Repository details changed while you were editing.', { exact: false }).waitFor()
    await page.getByRole('button', { name: 'Use updated details' }).click()
    assert.equal(await page.getByLabel('Description', { exact: true }).inputValue(), 'Changed elsewhere')

    const clone = page.getByRole('button', { name: 'Clone', exact: true })
    await clone.click()
    await page.getByRole('dialog').waitFor()
    await page.keyboard.press('Escape')
    assert.equal(await page.getByRole('dialog').count(), 0)
    assert.equal(await clone.evaluate(node => node === document.activeElement), true)
    await clone.click()
    await page.getByRole('button', { name: 'After clone' }).focus()
    assert.equal(await page.getByRole('dialog').count(), 0)

    await page.getByRole('button', { name: 'Delete repository', exact: true }).click()
    await page.getByRole('button', { name: 'Continue', exact: true }).click()
    await page.getByRole('alertdialog').getByRole('textbox').fill('demo')
    await page.getByRole('button', { name: 'Delete', exact: true }).click()
    await page.getByRole('alertdialog').getByRole('alert').filter({ hasText: 'Deletion denied by fixture' }).waitFor()
    await page.setViewportSize({ width: 390, height: 844 })
    const dialogBounds = await page.getByRole('alertdialog').boundingBox()
    assert(dialogBounds.x >= 0 && dialogBounds.x + dialogBounds.width <= 390)
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.dialog.png` })
    await page.setViewportSize({ width: 1280, height: 900 })
    await page.keyboard.press('Escape')

    const checks = page.locator('section').filter({ has: page.getByText('CI', { exact: true }) })
    await checks.getByText('Connected to octo/demo').waitFor()
    await checks.getByText('Connected by @owner on Jan 01, 2026, 12:00 AM UTC.').waitFor()
    assert.equal(await checks.getByRole('link', { name: 'octo/demo' }).getAttribute('href'), 'https://github.com/octo/demo')
    await page.getByRole('button', { name: 'GitHub uninstalled elsewhere', exact: true }).click()
    await checks.getByText('The Scope GitHub App was uninstalled from the GitHub account.').waitFor()
    await checks.getByRole('button', { name: 'Reconnect', exact: true }).waitFor()
    await page.evaluate(() => window.calls.splice(0))
    const becamePublic = { configured: true, connection: { github_full_name: 'octo/demo', github_url: 'https://github.com/octo/demo', connected_by: null, connected_at_unix: 1, disconnected: null, public_on_github: true, public_confirmed: false }, required_checks: [], can_confirm_public: true, setup_check: null, run_import_count: 50, run_import: null }
    await page.evaluate((github) => window.setFixtureGitHub(github), becamePublic)
    await checks.getByText('This GitHub repository became public, so Scope stopped sending private requests there.', { exact: false }).waitFor()
    await checks.getByRole('button', { name: 'Allow private requests', exact: true }).click()
    await checks.getByText('Public on GitHub: everything Scope pushes here is public.', { exact: true }).waitFor()
    assert.deepEqual(await page.evaluate(() => window.calls.splice(0)), [
      { confirmPublicGitHub: { owner: 'owner', repo: 'demo' } },
    ])
    const github = { configured: true, connection: { github_full_name: 'octo/demo', github_url: 'https://github.com/octo/demo', connected_by: null, connected_at_unix: 1, disconnected: null, public_on_github: false, public_confirmed: true }, required_checks: ['ci / test'], can_confirm_public: true, setup_check: null, run_import_count: 50, run_import: null }
    await page.evaluate((github) => window.setFixtureGitHub(github), github)
    await checks.getByText('ci / test', { exact: true }).waitFor()
    await checks.getByRole('textbox', { name: 'Check name' }).fill(' lint ')
    await checks.getByRole('button', { name: 'Add', exact: true }).click()
    assert.equal(await checks.getByRole('button', { name: 'Add', exact: true }).isDisabled(), true)
    await page.evaluate(() => window.finishAction('required-checks'))
    await checks.getByText('lint', { exact: true }).waitFor()
    assert.equal(await checks.getByRole('textbox', { name: 'Check name' }).inputValue(), '')
    await checks.getByRole('button', { name: 'Stop requiring ci / test', exact: true }).click()
    await page.evaluate(() => window.finishAction('required-checks'))
    await checks.getByText('ci / test', { exact: true }).waitFor({ state: 'detached' })
    assert.deepEqual(await page.evaluate(() => window.calls.splice(0)), [
      { setGitHubRequiredChecks: ['ci / test', 'lint'] },
      { setGitHubRequiredChecks: ['lint'] },
    ])

    await checks.getByText("branches: ['scope/**']", { exact: false }).waitFor()
    const testConnection = checks.getByRole('button', { name: 'Test connection', exact: true })
    await testConnection.click()
    assert.equal(await testConnection.isDisabled(), true)
    await page.evaluate(() => window.finishAction('setup-check'))
    await checks.getByText('Workflows ran on main (abcdef1). Choose the checks a request must pass.').waitFor()
    const ran = checks.getByRole('list', { name: 'Checks GitHub ran' })
    await ran.getByText('Required', { exact: true }).waitFor()
    const longName = 'test / unit and integration on every supported platform'
    await ran.getByRole('button', { name: `Require ${longName}`, exact: true }).click()
    await page.evaluate(() => window.finishAction('required-checks'))
    await ran.getByRole('button', { name: `Require ${longName}`, exact: true }).waitFor({ state: 'detached' })
    assert.deepEqual(await page.evaluate(() => window.calls.splice(0)), [
      { startGitHubSetupCheck: { owner: 'owner', repo: 'demo' } },
      { setGitHubRequiredChecks: ['lint', longName] },
    ])
    for (const [width, name] of [[1280, 'desktop'], [390, 'phone']]) {
      await page.setViewportSize({ width, height: 900 })
      const bounds = await checks.last().boundingBox()
      assert(bounds.x >= 0 && bounds.x + bounds.width <= width, `${name} checks fit`)
      if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
        await checks.last().screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.checks-${name}.png` })
      }
    }
    await page.setViewportSize({ width: 1280, height: 900 })

    const recentRuns = checks.getByRole('spinbutton', { name: 'Recent runs to import' })
    const save = checks.getByRole('button', { name: 'Save', exact: true })
    const importNow = checks.getByRole('button', { name: 'Import now', exact: true })
    assert.equal(await recentRuns.inputValue(), '50')
    assert.equal(await save.isDisabled(), true)
    await recentRuns.fill('1001')
    await checks.getByText('Enter a whole number from 0 to 1000.', { exact: true }).waitFor()
    assert.equal(await save.isDisabled(), true)
    await recentRuns.fill('200')
    await save.click()
    await page.evaluate(() => window.finishAction('run-import-count'))
    await page.waitForFunction(() => window.fixtureGitHub().run_import_count === 200)
    assert.equal(await recentRuns.inputValue(), '200')
    await importNow.click()
    await page.evaluate(() => window.finishAction('run-import'))
    await checks.getByText('Importing up to 200 runs from GitHub.', { exact: true }).waitFor()
    assert.equal(await importNow.isDisabled(), true)
    const imported = (runImport) => page.evaluate((runImport) => {
      const github = window.fixtureGitHub()
      window.setFixtureGitHub({ ...github, run_import: { ...github.run_import, ...runImport } })
    }, runImport)
    await imported({ error: 'GitHub answered 502 Bad Gateway for /repos/octo/demo/actions/runs: Server Error' })
    await checks.getByText('Import failed: GitHub answered 502 Bad Gateway for /repos/octo/demo/actions/runs: Server Error. Retrying.', { exact: true }).waitFor()
    assert.equal(await importNow.isDisabled(), false)
    for (const [width, name] of [[1280, 'desktop'], [390, 'phone']]) {
      await page.setViewportSize({ width, height: 900 })
      const bounds = await checks.last().boundingBox()
      assert(bounds.x >= 0 && bounds.x + bounds.width <= width, `${name} run import fits`)
      if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
        await recentRuns.scrollIntoViewIfNeeded()
        await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.run-import-${name}.png` })
      }
    }
    await page.setViewportSize({ width: 1280, height: 900 })
    await imported({ state: 'succeeded', error: null, imported_count: 200, finished_at_unix: 4 })
    await checks.getByText('Imported 200 runs.', { exact: true }).waitFor()
    assert.deepEqual(await page.evaluate(() => window.calls.splice(0)), [
      { setGitHubRunImportCount: 200 },
      { startGitHubRunImport: { owner: 'owner', repo: 'demo' } },
    ])

    await checks.getByRole('button', { name: 'Disconnect', exact: true }).click()
    assert.equal(await checks.getByRole('button', { name: 'Disconnect', exact: true }).isDisabled(), true)
    await page.evaluate(() => window.finishAction('disconnect-github'))
    await checks.getByText('Not connected to GitHub.').waitFor()
    await checks.getByRole('button', { name: 'Connect GitHub', exact: true }).click()
    await page.waitForFunction(() => location.hash === '#github-authorize')
    assert.deepEqual(await page.evaluate(() => window.calls.splice(0)), [
      { disconnectGitHub: { owner: 'owner', repo: 'demo' } },
      { startGitHubAuthorization: { owner: 'owner', repo: 'demo' } },
    ])

    const alice = page.getByRole('listitem').filter({ hasText: 'alice@example.com' })
    const bob = page.getByRole('listitem').filter({ hasText: 'bob@example.com' })
    await alice.getByRole('switch', { name: 'Change file visibility' }).click()
    await bob.getByRole('switch', { name: 'Push changes' }).click()
    assert.equal(await alice.getByRole('switch').first().isDisabled(), true)
    assert.equal(await bob.getByRole('switch').first().isDisabled(), true)
    await page.evaluate(() => window.finishAction('alice'))
    await page.waitForFunction(() => [...document.querySelectorAll('li')].find(node => node.textContent.includes('alice@example.com')).querySelector('[role="switch"]').disabled === false)
    assert.equal(await bob.getByRole('switch').first().isDisabled(), true)
    assert.equal(await page.evaluate(() => window.calls[0].permissions.can_change_file_visibility), true)
    assert.equal(await alice.getByRole('switch', { name: 'Change file visibility' }).isChecked(), true)
    await page.evaluate(() => window.finishAction('bob'))
    await alice.getByRole('switch', { name: 'Push changes' }).click()
    await bob.getByRole('switch', { name: 'Change file visibility' }).click()
    await page.evaluate(() => window.finishAction('bob'))
    assert.equal(await alice.getByRole('switch').first().isDisabled(), true)
    await page.evaluate(() => window.finishAction('alice'))
    assert.deepEqual(await page.evaluate(() => window.calls.filter(call => call.member_user_id === 'alice').at(-1).permissions), { can_push: true, can_change_file_visibility: true, view: 'private' })
    await bob.getByRole('combobox', { name: 'View' }).selectOption('public')
    await page.evaluate(() => window.finishAction('bob'))
    await page.waitForFunction(() => [...document.querySelectorAll('li')].find(node => node.textContent.includes('bob@example.com')).querySelector('select').value === 'public')
    assert.equal(await bob.getByRole('switch', { name: 'Push changes' }).isDisabled(), true)
    assert.deepEqual(await page.evaluate(() => window.calls.filter(call => call.member_user_id === 'bob').at(-1).permissions), { can_push: false, can_change_file_visibility: false, view: 'public' })

    await page.getByRole('button', { name: 'Create login command', exact: true }).click()
    await page.getByRole('button', { name: 'Revoke session-a', exact: true }).click()
    await page.getByRole('button', { name: 'Revoke session', exact: true }).click()
    await page.evaluate(() => window.finishAction('session-a'))
    assert.equal(await page.getByRole('button', { name: 'Create login command', exact: true }).isDisabled(), true)
    await page.evaluate(() => window.finishAction('grant'))
    await page.getByRole('button', { name: 'Create login command', exact: true }).click()
    await page.getByRole('button', { name: 'Revoke session-b', exact: true }).click()
    await page.getByRole('button', { name: 'Revoke session', exact: true }).click()
    await page.evaluate(() => window.finishAction('grant'))
    assert.equal(await page.getByRole('button', { name: 'Revoke session-b', exact: true }).isDisabled(), true)
    await page.evaluate(() => window.finishAction('session-b'))

    await page.getByRole('button', { name: 'Invite member', exact: true }).click()
    await page.getByLabel('Email address').fill('new@example.com')
    await page.getByRole('button', { name: 'Send invitation', exact: true }).click()
    await page.getByRole('dialog').waitFor({ state: 'detached' })
    const invitation = page.locator('main li', { hasText: 'new@example.com' })
    await invitation.getByText(/^Email sent · Expires /).waitFor()
    assert.deepEqual(await page.evaluate(() => window.calls.at(-1)), {
      email: 'new@example.com', owner: 'owner', repo: 'demo',
      permissions: { can_push: false, can_change_file_visibility: false, view: 'private' },
    })
    await invitation.getByRole('button', { name: 'Copy link', exact: true }).click()
    await page.getByText('https://example.com/invites/new-token', { exact: true }).waitFor()
    await page.setViewportSize({ width: 390, height: 844 })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.screenshot({ path: process.env.SCOPE_COMPONENT_SCREENSHOT, fullPage: true })
    await page.setViewportSize({ width: 1024, height: 768 })
    await page.locator('iframe').waitFor()
    const first = await page.locator('iframe').elementHandle()
    const source = page.locator('pre').filter({ hasText: '<h1>Test preview</h1>' })
    await page.getByRole('button', { name: 'Toggle source' }).click()
    await source.waitFor()
    await page.getByRole('button', { name: 'Toggle source' }).click()
    await source.waitFor({ state: 'detached' })
    assert.equal(await first.evaluate(node => node === document.querySelector('iframe')), true)
    await page.getByRole('button', { name: 'Toggle theme' }).click()
    await page.frameLocator('iframe').getByRole('heading', { name: 'Test preview' }).waitFor()
    assert.equal(await first.evaluate(node => node.isConnected), false)
    assert.equal(await page.locator('iframe').count(), 1)
    const close = page.getByTitle('Close Two.html', { exact: true })
    await close.locator('..').hover()
    await page.waitForFunction(() => [...document.querySelectorAll('button')].some((node) =>
      node.title === 'Close Two.html' &&
      getComputedStyle(node).opacity === '1'))
    assert.equal(await close.evaluate(node => getComputedStyle(node).opacity), '1')
    await close.tap()
    assert.equal(await page.getByRole('tab', { name: 'Two.html' }).count(), 0)
    await page.getByLabel('Description', { exact: true }).fill('Draft for demo only')
    await page.getByRole('button', { name: 'Other repository settings', exact: true }).click()
    assert.equal(await page.getByLabel('Description', { exact: true }).inputValue(), 'Other repository description')
    await page.getByRole('button', { name: 'Save details', exact: true }).click()
    await page.getByText('Details saved.', { exact: true }).waitFor()
    assert.deepEqual(await page.evaluate(() => window.calls.at(-1)), {
      owner: 'owner', repo: 'other', description: 'Other repository description', website_url: '',
    })

    assert.equal(await page.getByLabel('Selected revision').textContent(), 'server-seed')
    assert.equal(await page.getByLabel('Revision loads').textContent(), '0')
    await page.getByRole('button', { name: 'Toggle changes', exact: true }).click()
    await page.getByRole('button', { name: 'Toggle changes', exact: true }).click()
    assert.equal(await page.getByLabel('Selected revision').textContent(), 'server-seed')
    assert.equal(await page.getByLabel('Revision loads').textContent(), '0')
    await page.getByRole('button', { name: 'Invalidate changes', exact: true }).click()
    await page.getByLabel('Selected revision').filter({ hasText: 'user_clerk_original:private' }).waitFor()
    assert.equal(await page.getByLabel('Revision loads').textContent(), '1')
    await page.getByRole('button', { name: 'Switch changes viewer', exact: true }).click()
    await page.getByLabel('Selected revision').filter({ hasText: 'user_clerk_other:private' }).waitFor()
    assert.equal(await page.getByLabel('Revision loads').textContent(), '2')
    await page.getByRole('button', { name: 'Switch changes access', exact: true }).click()
    await page.getByLabel('Selected revision').filter({ hasText: 'user_clerk_other:public' }).waitFor()
    assert.equal(await page.getByLabel('Revision loads').textContent(), '3')
    assert.deepEqual(errors, [])
  } finally {
    await browser.close()
    await server.close()
  }
})
