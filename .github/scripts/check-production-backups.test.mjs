import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import { assessBackups, fetchBackupHealth } from './check-production-backups.mjs'

const now = Date.parse('2026-09-15T18:00:00Z')
const valid = () => ({
  volumeInstanceBackupScheduleList: [{ kind: 'DAILY' }, { kind: 'WEEKLY' }],
  volumeInstanceBackupList: [{ createdAt: '2026-09-15T16:01:14.247Z', expiresAt: '2026-10-15T16:01:14.247Z' }],
})

test('scheduled backups with a recent retained snapshot are healthy', () => {
  assert.equal(assessBackups(valid(), now).healthy, true)
})
test('missing schedule, stale snapshots and empty snapshots fail independently', () => {
  const noSchedule = valid()
  noSchedule.volumeInstanceBackupScheduleList = [{ kind: 'DAILY' }]
  assert.match(assessBackups(noSchedule, now).failures.join(), /WEEKLY/)
  const stale = valid()
  stale.volumeInstanceBackupList[0].createdAt = '2026-09-14T00:00:00Z'
  assert.match(assessBackups(stale, now).failures.join(), /42 hours/)
  const empty = valid()
  empty.volumeInstanceBackupList = []
  assert.equal(assessBackups(empty, now).healthy, false)
})
test('expired and future dated snapshots cannot mask backup failure', () => {
  const data = valid()
  data.volumeInstanceBackupList = [
    { createdAt: '2026-09-15T16:00:00Z', expiresAt: '2026-09-15T17:00:00Z' },
    { createdAt: '2026-09-16T16:00:00Z' },
  ]
  assert.equal(assessBackups(data, now).healthy, false)
})
test('query uses project token and rejects API errors without echoing sensitive details', async () => {
  await assert.rejects(fetchBackupHealth('secret-token', async (url, options) => {
    assert.equal(options.headers['project-access-token'], 'secret-token')
    assert.equal(options.headers.authorization, undefined)
    return { ok: true, json: async () => ({ errors: [{ message: 'sensitive internal detail' }] }) }
  }), error => !error.message.includes('sensitive internal detail') && /rejected/.test(error.message))
  await assert.rejects(fetchBackupHealth('', async () => { throw new Error('must not fetch') }), /missing/)
})

test('scheduled monitor does not inherit every repository secret', () => {
  const caller = readFileSync(new URL('../workflows/backup-monitor.yml', import.meta.url), 'utf8')
  assert.doesNotMatch(caller, /secrets: inherit/)
})
