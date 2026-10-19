import assert from 'node:assert/strict'
import test from 'node:test'
import {
  initialMediaRecovery,
  type MediaRecovery,
  recoverByRetry,
  recoverFromMediaError,
  recoverFromMediaLoad,
} from './request-attachment-media-recovery'

type Step = ['error', string] | ['load', string] | ['retry']

function replay(steps: Step[]) {
  let recovery: MediaRecovery = initialMediaRecovery
  const renewals: string[] = []
  for (const step of steps) {
    if (step[0] === 'error') {
      const next = recoverFromMediaError(recovery, step[1])
      recovery = next.recovery
      if (next.renew) renewals.push(step[1])
    } else if (step[0] === 'load') {
      recovery = recoverFromMediaLoad(recovery, step[1])
    } else {
      recovery = recoverByRetry(recovery)
    }
  }
  return { failed: recovery.failed, renewals }
}

const cases: Array<[string, Step[], { failed: string | null; renewals: string[] }]> = [
  ['renews an errored grant once and fails when the fresh grant also errors',
    [['error', 'a'], ['error', 'b']], { failed: 'b', renewals: ['a'] }],
  ['ignores repeated errors from the grant being replaced',
    [['error', 'a'], ['error', 'a'], ['load', 'b']], { failed: null, renewals: ['a'] }],
  ['renews again after the fresh grant loaded and later expired',
    [['error', 'a'], ['load', 'b'], ['error', 'b'], ['load', 'c']], { failed: null, renewals: ['a', 'b'] }],
  ['a manual retry does not automatically renew when its grant errors',
    [['error', 'a'], ['error', 'b'], ['retry'], ['error', 'b'], ['error', 'c']], { failed: 'c', renewals: ['a'] }],
  ['a manual retry that loads restores automatic renewal',
    [['error', 'a'], ['error', 'b'], ['retry'], ['load', 'c'], ['error', 'c']], { failed: null, renewals: ['a', 'c'] }],
]

for (const [name, steps, expected] of cases) {
  test(name, () => {
    assert.deepEqual(replay(steps), expected)
  })
}
