import assert from 'node:assert/strict'
import test from 'node:test'
import { BoundedDelivery } from './delivery'

const event = { event: '$pageview', properties: { distinct_id: 'anonymous' } }

test('queue and payload limits drop excess capture without waiting for delivery', () => {
  const delivery = new BoundedDelivery({ fetcher: async () => new Promise<Response>(() => {}) })
  for (let index = 0; index < 65; index++) assert.equal(delivery.enqueue(event), true)
  assert.equal(delivery.enqueue(event), false)
  assert.equal(delivery.enqueue({ value: 'x'.repeat(16 * 1024) }), false)
  delivery.flushOnPageHide()
})

test('temporary rejection is retried, while permanent rejection is dropped', async () => {
  const statuses = [503, 200, 400]
  const attempts: number[] = []
  const delivery = new BoundedDelivery({
    fetcher: async (_url, init) => {
      attempts.push(statuses.shift()!)
      assert.equal(init?.credentials, 'omit')
      assert.equal(init?.referrerPolicy, 'no-referrer')
      return new Response(null, { status: attempts.at(-1) })
    },
  })
  delivery.enqueue(event)
  delivery.enqueue(event)
  await until(() => attempts.length === 3)
  assert.deepEqual(attempts, [503, 200, 400])
})

test('offline delivery stops after the retry cap', async () => {
  let attempts = 0
  const delivery = new BoundedDelivery({
    fetcher: async () => {
      attempts++
      throw new Error('offline')
    },
  })
  delivery.enqueue(event)
  await until(() => attempts === 3)
  await new Promise(resolve => setTimeout(resolve, 300))
  assert.equal(attempts, 3)
})

test('reset drops queued events and aborts a failed request before retry', async () => {
  const attempts: string[] = []
  let aborted = false
  const delivery = new BoundedDelivery({
    fetcher: async (_url, init) => {
      const payload = JSON.parse(init?.body as string) as { event: string }
      attempts.push(payload.event)
      if (payload.event === 'old') {
        return new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener('abort', () => {
            aborted = true
            reject(new Error('aborted'))
          }, { once: true })
        })
      }
      return Response.json({ status: 1 })
    },
  })
  delivery.enqueue({ event: 'old' })
  delivery.enqueue({ event: 'queued-old' })
  delivery.clear()
  delivery.enqueue({ event: 'new' })
  await until(() => attempts.includes('new'))
  assert.equal(aborted, true)
  assert.deepEqual(attempts, ['old', 'new'])
})

test('page hide offers active and queued events to sendBeacon with their original identity', async () => {
  const sent: Blob[] = []
  let aborted = false
  const delivery = new BoundedDelivery({
    fetcher: async (_url, init) => new Promise<Response>((_resolve, reject) => {
      init?.signal?.addEventListener('abort', () => {
        aborted = true
        reject(new Error('aborted'))
      }, { once: true })
    }),
    beacon: (_url, payload) => { sent.push(payload); return true },
  })
  const active = { ...event, uuid: 'active-event', timestamp: '2026-09-23T12:00:00Z' }
  const queued = { ...event, uuid: 'queued-event', timestamp: '2026-09-23T12:00:01Z' }
  delivery.enqueue(active)
  delivery.enqueue(queued)
  delivery.flushOnPageHide()
  assert.equal(aborted, true)
  assert.deepEqual(await Promise.all(sent.map(async payload => JSON.parse(await payload.text()))), [active, queued])
  assert.equal(delivery.enqueue(event), false)
  delivery.resume()
  assert.equal(delivery.enqueue(event), true)
  delivery.clear()
})

test('reset prevents active and queued identity data from reaching a later page-hide beacon', () => {
  const sent: Blob[] = []
  const delivery = new BoundedDelivery({
    fetcher: async () => new Promise<Response>(() => {}),
    beacon: (_url, payload) => { sent.push(payload); return true },
  })
  delivery.enqueue(event)
  delivery.enqueue(event)
  delivery.clear()
  delivery.flushOnPageHide()
  assert.deepEqual(sent, [])
})

async function until(condition: () => boolean) {
  const deadline = Date.now() + 2_000
  while (!condition()) {
    assert.ok(Date.now() < deadline, 'delivery did not reach expected state')
    await new Promise(resolve => setTimeout(resolve, 10))
  }
}
