import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import test from 'node:test'
import { sha256Blob } from './request-attachment-hash'

test('hashes a blob incrementally across part boundaries', async () => {
  assert.equal(
    await sha256Blob(new Blob(['abc']), 1),
    'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
  )
})

test('matches Node SHA-256 across padding and multi-block boundaries', async () => {
  for (const size of [0, 55, 56, 63, 64, 65, 1024 * 1024 + 17]) {
    const bytes = Uint8Array.from(
      { length: size },
      (_, index) => (index * 131 + 17) % 256,
    )
    const expected = createHash('sha256').update(bytes).digest('hex')
    assert.equal(await sha256Blob(new Blob([bytes]), 8 * 1024), expected, `size ${size}`)
  }
})

test('hashes content spanning the preferred eight MiB transfer part', async () => {
  const bytes = new Uint8Array(8 * 1024 * 1024 + 17)
  for (let index = 0; index < bytes.length; index += 4093) bytes[index] = index % 251
  const expected = createHash('sha256').update(bytes).digest('hex')
  assert.equal(await sha256Blob(new Blob([bytes])), expected)
})
