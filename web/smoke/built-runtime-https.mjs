import assert from 'node:assert/strict'
import { spawn, execFileSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { createServer as createHttpServer, request as httpRequest } from 'node:http'
import { createServer as createHttpsServer } from 'node:https'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { chromium, request as playwrightRequest } from 'playwright'

const webRoot = new URL('..', import.meta.url)
const builtServer = new URL('../.output/server/index.mjs', import.meta.url)

async function freePort() {
  const server = createHttpServer()
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const port = server.address().port
  await new Promise((resolve) => server.close(resolve))
  return port
}

async function ready(port, child) {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline && child.exitCode === null) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/readyz`, { signal: AbortSignal.timeout(2_000) })
      await response.body?.cancel()
      if (response.status < 500) return
    } catch {}
    await delay(100)
  }
  throw new Error(`built web server did not start (exit ${child.exitCode ?? 'pending'})`)
}

async function close(server) {
  server.closeAllConnections()
  await new Promise((resolve) => server.close(resolve))
}

test('compiled web server accepts same-origin HTTPS and rejects hostile origins behind TLS termination', { timeout: 90_000 }, async () => {
  const temporary = mkdtempSync(join(tmpdir(), 'scope-https-smoke-'))
  let child
  let proxy
  let browser
  let api
  let output = ''
  try {
    const key = join(temporary, 'key.pem')
    const certificate = join(temporary, 'certificate.pem')
    execFileSync('openssl', [
      'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
      '-keyout', key, '-out', certificate, '-subj', '/CN=localhost',
      '-addext', 'subjectAltName=DNS:localhost,IP:127.0.0.1',
    ], { stdio: 'ignore' })
    const upstreamPort = await freePort()
    child = spawn(process.execPath, [fileURLToPath(builtServer)], {
      cwd: fileURLToPath(webRoot),
      env: {
        ...process.env,
        NODE_ENV: 'production',
        HOST: '127.0.0.1',
        PORT: String(upstreamPort),
        RAILWAY_ENVIRONMENT_ID: 'scope-https-smoke',
        CLERK_SECRET_KEY: process.env.CLERK_SECRET_KEY ?? 'sk_test_scope_e2e',
        SCOPE_API_INTERNAL_URL: process.env.SCOPE_API_INTERNAL_URL ?? 'http://127.0.0.1:8080',
        SCOPE_API_PUBLIC_URL: process.env.SCOPE_API_PUBLIC_URL ?? 'http://127.0.0.1:8080',
        SCOPE_CLI_INSTALL_URL: process.env.SCOPE_CLI_INSTALL_URL ?? 'https://scope.example/install.sh',
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    for (const stream of [child.stdout, child.stderr]) {
      stream.on('data', (chunk) => { output = (output + chunk).slice(-8_000) })
    }
    await ready(upstreamPort, child)

    proxy = createHttpsServer({ key: readFileSync(key), cert: readFileSync(certificate) }, (incoming, outgoing) => {
      const upstream = httpRequest({
        hostname: '127.0.0.1',
        port: upstreamPort,
        path: incoming.url,
        method: incoming.method,
        headers: {
          ...incoming.headers,
          'x-forwarded-proto': 'https',
          'x-forwarded-host': incoming.headers.host,
        },
      }, (response) => {
        outgoing.writeHead(response.statusCode, response.headers)
        response.pipe(outgoing)
      })
      upstream.on('error', (error) => {
        outgoing.writeHead(502)
        outgoing.end(error.message)
      })
      incoming.pipe(upstream)
    })
    await new Promise((resolve) => proxy.listen(0, '127.0.0.1', resolve))
    const baseUrl = `https://127.0.0.1:${proxy.address().port}`
    api = await playwrightRequest.newContext({ ignoreHTTPSErrors: true, timeout: 5_000 })
    browser = await chromium.launch({ headless: true })
    const page = await browser.newPage({ ignoreHTTPSErrors: true })
    const navigation = await page.goto(baseUrl, { waitUntil: 'domcontentloaded' })
    assert(navigation && navigation.status() < 400,
      `compiled page returned ${navigation?.status()} through HTTPS: ${(await navigation?.text())?.slice(-2_000)}; server: ${output}`)
    const browserStatus = await page.evaluate(async () =>
      (await fetch('/_serverFn/invalid', { method: 'POST', body: '{}', signal: AbortSignal.timeout(5_000) })).status)
    assert.notEqual(browserStatus, 403, 'same-origin HTTPS browser request was rejected')

    const endpoint = `${baseUrl}/_serverFn/invalid`
    for (const headers of [
      { Origin: baseUrl },
      { Referer: `${baseUrl}/` },
      { 'Sec-Fetch-Site': 'same-origin' },
    ]) {
      const response = await api.get(endpoint, { headers })
      assert.notEqual(response.status(), 403, JSON.stringify(headers))
    }
    for (const headers of [
      { Origin: `http://127.0.0.1:${proxy.address().port}` },
      { Origin: 'https://hostile.example' },
      { Origin: `${baseUrl}.hostile.example` },
      { Referer: 'https://hostile.example/path' },
      { Origin: 'https://hostile.example', 'x-forwarded-host': `127.0.0.1:${proxy.address().port}`, 'x-forwarded-proto': 'https' },
    ]) {
      const response = await api.get(endpoint, { headers })
      assert.equal(response.status(), 403, JSON.stringify(headers))
    }
  } catch (error) {
    error.message += `\nBuilt server output:\n${output}`
    throw error
  } finally {
    await api?.dispose()
    await browser?.close()
    if (proxy?.listening) await close(proxy)
    if (child && child.exitCode === null) {
      child.kill('SIGTERM')
      let killTimer
      try {
        await Promise.race([
          new Promise((resolve) => child.once('exit', resolve)),
          new Promise((resolve) => {
            killTimer = setTimeout(() => { child.kill('SIGKILL'); resolve() }, 3_000)
          }),
        ])
      } finally {
        clearTimeout(killTimer)
      }
    }
    rmSync(temporary, { recursive: true, force: true })
  }
})
