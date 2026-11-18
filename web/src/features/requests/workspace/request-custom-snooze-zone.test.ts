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

test('local snooze fields do not shift through UTC', async () => {
  const { localSnoozeDate, localSnoozeTime } = await import('./request-workspace-model')
  const date = new Date(2026, 0, 2, 20, 4)
  assert.equal(localSnoozeDate(date), '2026-01-02')
  assert.equal(localSnoozeTime(date), '20:04')
})
