import assert from 'node:assert/strict'
import test from 'node:test'
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
