import assert from 'node:assert/strict'
import { brotliDecompressSync, createBrotliDecompress, createGunzip, gunzipSync } from 'node:zlib'
import { Readable } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import type { ReadableStream as NodeReadableStream } from 'node:stream/web'
import test from 'node:test'
import { compressResponse } from './response-compression'

const source = JSON.stringify({ text: 'Scope source control\n'.repeat(200) })

function request(encoding: string, method = 'GET') {
  return new Request('https://scope.test/data', {
    headers: { 'accept-encoding': encoding }, method,
  })
}

function response(headers: Record<string, string> = {}) {
  return new Response(source, {
    headers: { 'content-type': 'application/json', ...headers },
  })
}

test('negotiates encodings, round trips content and preserves Vary', async () => {
  for (const [accepted, expected] of [
    ['gzip, br', 'br'], ['br;q=0.5,gzip;q=1', 'gzip'], ['*', 'br'],
    ['br;q=0,gzip;q=0', null], ['br;q=invalid', null],
  ] as const) {
    const result = compressResponse(request(accepted), response({ vary: 'Cookie' }))
    assert.equal(result.headers.get('content-encoding'), expected)
    assert.equal(result.headers.get('vary'), 'Cookie, Accept-Encoding')
    const bytes = Buffer.from(await result.arrayBuffer())
    const decoded = expected === 'br' ? brotliDecompressSync(bytes)
      : expected === 'gzip' ? gunzipSync(bytes) : bytes
    assert.equal(decoded.toString(), source)
  }
})

test('leaves known tiny bodies and excluded responses alone', () => {
  for (const headers of [
    { 'content-length': '123' },
    { 'content-encoding': 'gzip' },
    { 'cache-control': 'no-transform' },
    { 'content-type': 'text/event-stream' },
    { 'content-type': 'application/octet-stream' },
  ] as Array<Record<string, string>>) {
    const original = response(headers)
    assert.equal(compressResponse(request('br'), original), original)
  }
  const original = response()
  assert.equal(compressResponse(request('br', 'HEAD'), original), original)
  for (const status of [204, 304]) {
    const empty = new Response(null, { status })
    assert.equal(compressResponse(request('br'), empty), empty)
  }
})

for (const encoding of ['br', 'gzip']) {
  test(`${encoding} delivers HTML before its source finishes`, { timeout: 5_000 }, async () => {
    const shell = '<!doctype html><main>Loading repository…</main>'
    const tail = '<script>/* deferred route data */</script>'
    let input!: ReadableStreamDefaultController<Uint8Array>
    const body = new ReadableStream<Uint8Array>({
      start(controller) { input = controller },
    })
    const result = compressResponse(request(encoding), new Response(body, {
      headers: { 'content-type': 'text/html' },
    }))
    const decoder = encoding === 'br' ? createBrotliDecompress() : createGunzip()
    let decoded = ''
    let resolveShell!: () => void
    const receivedShell = new Promise<void>((resolve) => { resolveShell = resolve })
    decoder.on('data', (chunk: Buffer) => {
      decoded += chunk.toString()
      if (decoded === shell) resolveShell()
    })
    const complete = pipeline(
      Readable.fromWeb(result.body as unknown as NodeReadableStream), decoder,
    )
    let timeout: ReturnType<typeof setTimeout> | undefined
    try {
      input.enqueue(new TextEncoder().encode(shell))
      await Promise.race([
        receivedShell,
        new Promise<never>((_resolve, reject) => {
          timeout = setTimeout(() => reject(new Error('HTML shell was buffered until source close')), 2_000)
        }),
      ])
      assert.equal(decoded, shell)
      input.enqueue(new TextEncoder().encode(tail))
    } finally {
      clearTimeout(timeout)
      input.close()
      await complete
    }
    assert.equal(decoded, shell + tail)
  })
}
