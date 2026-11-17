import assert from 'node:assert/strict'
import test from 'node:test'
import { runAttachmentHashWorker } from './request-attachment-hash-client'

function hashWorker() {
  return {
    onmessage: null as Worker['onmessage'],
    onerror: null as Worker['onerror'],
    postMessage: () => {},
    terminated: false,
    terminate() { this.terminated = true },
  }
}

test('cancelling a hash immediately terminates its worker', async () => {
  const controller = new AbortController()
  const worker = hashWorker()
  const pending = runAttachmentHashWorker(new Blob(['video']), controller.signal, worker)
  controller.abort()
  await assert.rejects(pending, { name: 'AbortError' })
  assert.equal(worker.terminated, true)
  assert.equal(worker.onmessage, null)
})

test('a completed hash terminates its worker and detaches cancellation', async () => {
  const controller = new AbortController()
  const worker = hashWorker()
  const pending = runAttachmentHashWorker(new Blob(['video']), controller.signal, worker)
  worker.onmessage?.call(worker as unknown as Worker, new MessageEvent('message', { data: { sha256: 'a'.repeat(64) } }))
  assert.equal(await pending, 'a'.repeat(64))
  assert.equal(worker.terminated, true)
  controller.abort()
})
