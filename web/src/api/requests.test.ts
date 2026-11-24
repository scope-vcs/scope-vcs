import assert from 'node:assert/strict'
import test from 'node:test'
import { parseLoadRequestBacklogInput } from './request-backlog-input'
import { parseLoadRequestQueueInput, type LoadRequestQueueInput } from './request-queue-input'

test('parseLoadRequestQueueInput normalizes pagination, search and empty optional values', () => {
  const sections = ['active', 'unclaimed', 'set_aside', 'done'] as const
  const cases: [unknown, LoadRequestQueueInput][] = [
    [
      { cursor: '  open:page-2  ', owner: ' scope ', repo: ' vcs ', search: '  atomic refs  ', section: 'active', view: 'agent' },
      { cursor: 'open:page-2', owner: 'scope', repo: 'vcs', search: 'atomic refs', section: 'active', view: 'agent' },
    ],
    [
      { cursor: ' ', owner: 'scope', repo: 'vcs', search: '\n', section: 'set_aside', view: '' },
      { cursor: null, owner: 'scope', repo: 'vcs', search: null, section: 'set_aside', view: null },
    ],
    ...sections.map((section): [unknown, LoadRequestQueueInput] => [
      { owner: 'scope', repo: 'vcs', section, search: 'needle' },
      { cursor: null, owner: 'scope', repo: 'vcs', search: 'needle', section, view: null },
    ]),
  ]
  for (const [input, expected] of cases) assert.deepEqual(parseLoadRequestQueueInput(input), expected)
})

test('parseLoadRequestQueueInput rejects malformed views', () => {
  assert.throws(
    () => parseLoadRequestQueueInput({ owner: 'scope', repo: 'vcs', section: 'active', view: 'Agent' }),
    /Unsupported view/,
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

test('parseLoadRequestBacklogInput keeps a whole positive page size and drops any other', () => {
  const sized = (limit: unknown) => parseLoadRequestBacklogInput({ owner: 'scope', repo: 'vcs', limit }).limit
  assert.equal(sized(1), 1)
  for (const limit of [0, -1, 1.5, '1', undefined]) assert.equal(sized(limit), null)
})
