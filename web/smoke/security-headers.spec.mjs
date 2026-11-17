import assert from 'node:assert/strict'
import { test } from 'node:test'
import { baseUrl, withPage } from './browser-smoke.mjs'

test('web responses protect framing and refuse injected scripts while sandboxed previews and blob images render', async () => {
  await withPage('/', async (page) => {
    const response = await page.request.get(`${baseUrl}/`)
    const policy = response.headers()['content-security-policy']
    assert.match(policy, /frame-ancestors 'none'/)
    assert.match(policy, /default-src 'self'/)
    assert.match(policy, /script-src 'self' 'nonce-[A-Za-z0-9+/=]+'/)
    assert.doesNotMatch(policy, /script-src[^;]*'unsafe-(inline|eval)'/)
    assert.equal(response.headers()['x-content-type-options'], 'nosniff')
    assert.equal(response.headers()['x-frame-options'], 'DENY')

    const blocked = await page.evaluate(() => new Promise((resolve) => {
      document.addEventListener('securitypolicyviolation', (event) => resolve(event.effectiveDirective), { once: true })
      const script = document.createElement('script')
      script.textContent = 'window.injectedScriptRan = true'
      document.head.append(script)
    }))
    assert.equal(blocked, 'script-src-elem')
    assert.equal(await page.evaluate(() => window.injectedScriptRan), undefined)

    await page.evaluate(() => {
      const frame = document.createElement('iframe')
      frame.title = 'Security preview fixture'
      frame.setAttribute('sandbox', '')
      frame.srcdoc = '<style>h1{color:rgb(1, 2, 3)}</style><h1>Preview rendered</h1><script>document.body.textContent="Unsafe script executed"</script>'
      document.body.append(frame)
      const image = document.createElement('img')
      image.alt = 'Security blob fixture'
      image.src = URL.createObjectURL(new Blob([
        '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="red"/></svg>',
      ], { type: 'image/svg+xml' }))
      document.body.append(image)
    })
    const heading = page.frameLocator('iframe[title="Security preview fixture"]').getByRole('heading', { name: 'Preview rendered' })
    await heading.waitFor()
    assert.equal(await heading.evaluate((element) => getComputedStyle(element).color), 'rgb(1, 2, 3)')
    await page.waitForFunction(() => document.querySelector('img[alt="Security blob fixture"]')?.naturalWidth === 10)
  })
})
