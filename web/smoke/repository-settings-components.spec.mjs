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
test('settings preserve drafts, report deletion errors, release invite results, and track concurrent rows', async (t) => {
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
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
  page.setDefaultTimeout(10000)
  try {
    const errors = []
    page.on('pageerror', error => errors.push(error.message))
    await page.goto(server.resolvedUrls.local[0], { timeout: 30_000, waitUntil: 'domcontentloaded' })
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
    assert.deepEqual(await page.evaluate(() => window.calls.filter(call => call.member_user_id === 'alice').at(-1).permissions), { can_push: true, can_change_file_visibility: true })

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

    await page.getByLabel('Member email').fill('new@example.com')
    await page.getByRole('button', { name: 'Invite', exact: true }).click()
    await page.getByText('https://example.com/invites/new-token', { exact: true }).waitFor()
    assert.equal(await page.getByLabel('Member email').isEnabled(), true)
    await page.setViewportSize({ width: 390, height: 844 })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.screenshot({ path: process.env.SCOPE_COMPONENT_SCREENSHOT, fullPage: true })
    assert.deepEqual(errors, [])
  } catch (error) {
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.failure.png`, fullPage: true })
    throw error
  } finally {
    await browser.close()
    await server.close()
  }
})
