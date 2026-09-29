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

// The server keeps a reply the browser saw fail and answers a repeated
// client_reply_id with that stored reply, so a changed quote needs a new id.
test('a reply resent with a different quote is a new attempt', async (t) => {
  const cacheDir = await mkdtemp(join(tmpdir(), 'scope-vite-request-reply-'))
  t.after(() => rm(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    cacheDir,
    configFile: false,
    root: fileURLToPath(new URL('./fixtures/request-reply', import.meta.url)),
    server: {
      host: '127.0.0.1',
      port: 0,
      fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] },
    },
    resolve: { alias: [
      { find: '@clerk/tanstack-react-start', replacement: fileURLToPath(new URL('./fixtures/request-reply/clerk.ts', import.meta.url)) },
      { find: '@', replacement: fileURLToPath(new URL('../src', import.meta.url)) },
      ...['react/jsx-dev-runtime', 'react/jsx-runtime', 'react-dom/client', 'react']
        .map((name) => ({ find: name, replacement: require.resolve(name) })),
    ] },
    oxc: { jsx: { runtime: 'automatic' } },
  })
  await server.listen()
  t.after(() => server.close())
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
  page.setDefaultTimeout(30_000)
  const errors = []
  page.on('pageerror', (error) => errors.push(error.message))
  await page.goto(server.resolvedUrls.local[0], { waitUntil: 'domcontentloaded' })

  const composer = page.locator('form')
  const send = composer.getByRole('button', { name: 'Reply', exact: true })
  await page.getByRole('button', { name: 'Reply to alice' }).click()
  await composer.getByText('Cap retries at five.').waitFor()
  await composer.getByRole('textbox', { name: 'Reply' }).fill('Five matches the client default.')
  await send.click()
  await page.getByRole('alert').filter({ hasText: 'Service unavailable' }).waitFor()

  await page.getByRole('button', { name: 'Reply to bob' }).click()
  await composer.getByText('Cap retries at three.').waitFor()
  await send.click()
  await page.waitForFunction(() => window.calls.length === 2)
  await composer.waitFor({ state: 'detached' })

  const [failed, resent] = await page.evaluate(() => window.calls)
  assert.equal(failed.reply_to_reply_id, 'reply-alice')
  assert.equal(resent.reply_to_reply_id, 'reply-bob')
  assert.equal(resent.body_markdown, failed.body_markdown)
  assert.notEqual(resent.client_reply_id, failed.client_reply_id)
  assert.deepEqual(errors, [])
})
