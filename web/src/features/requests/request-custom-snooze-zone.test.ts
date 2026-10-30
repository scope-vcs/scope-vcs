import assert from 'node:assert/strict'
import test from 'node:test'

process.env.TZ = 'America/Los_Angeles'

test('a custom date inside the spring-forward gap is rejected', async () => {
  const { resolveCustomSnooze } = await import('./request-workspace-model')
  assert.equal(
    resolveCustomSnooze({ mode: 'date', date: '2027-03-14', time: '02:30' }, new Date('2027-03-01T00:00:00Z')),
    null,
  )
})
