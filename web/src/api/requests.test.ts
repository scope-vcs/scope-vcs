import assert from 'node:assert/strict'
import test from 'node:test'
import { parseLoadRequestQueueInput } from './request-queue-input'

test('parseLoadRequestQueueInput normalizes pagination and search', () => {
  assert.deepEqual(
    parseLoadRequestQueueInput({
      cursor: '  open:page-2  ',
      owner: ' scope ',
      repo: ' vcs ',
      search: '  atomic refs  ',
      section: 'active',
    }),
    {
      cursor: 'open:page-2',
      owner: 'scope',
      repo: 'vcs',
      search: 'atomic refs',
      section: 'active',
    },
  )
})

test('parseLoadRequestQueueInput rejects unknown sections', () => {
  assert.throws(
    () =>
      parseLoadRequestQueueInput({
        owner: 'scope',
        repo: 'vcs',
        section: 'everything',
      }),
    /section is invalid/,
  )

})

test('parseLoadRequestQueueInput removes empty optional values', () => {
  assert.deepEqual(
    parseLoadRequestQueueInput({
      cursor: ' ',
      owner: 'scope',
      repo: 'vcs',
      search: '\n',
      section: 'set_aside',
    }),
    {
      cursor: null,
      owner: 'scope',
      repo: 'vcs',
      search: null,
      section: 'set_aside',
    },
  )
})

test('each attention section supports scoped search', () => {
  for (const section of ['active', 'unclaimed', 'set_aside']) {
    assert.equal(parseLoadRequestQueueInput({ owner: 'scope', repo: 'vcs', section, search: 'needle' }).search, 'needle')
  }
})
