import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { once } from 'node:events'
import test from 'node:test'
import { sampleRequest, summarize } from './media-capacity-samples.mjs'

test('a 200 response only passes once its entire body completes', async (t) => {
  const server = createServer((req, res) => {
    res.writeHead(200, { 'content-length': 10 })
    res.write('short')
    if (req.url === '/truncated') setTimeout(() => res.destroy(), 10)
    else if (req.url === '/complete') res.end(' body')
  })
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  t.after(() => { server.closeAllConnections(); server.close() })
  const origin = `http://127.0.0.1:${server.address().port}`
  const truncated = await sampleRequest(`${origin}/truncated`, 'test', null)
  assert.equal(truncated.status, 200)
  assert.equal(truncated.completed, false)
  assert.ok(truncated.error)
  assert.equal(summarize([truncated]).failed_requests, 1)
  const timeout = await sampleRequest(`${origin}/timeout`, 'test', null, 100)
  assert.equal(timeout.status, 200)
  assert.equal(timeout.completed, false)
  assert.equal(summarize([timeout]).failed_requests, 1)
  const complete = await sampleRequest(`${origin}/complete`, 'test', null)
  assert.equal(complete.completed, true)
  assert.equal(complete.error, null)
  assert.equal(summarize([complete]).failed_requests, 0)
})
