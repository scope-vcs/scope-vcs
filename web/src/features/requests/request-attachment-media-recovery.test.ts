import assert from 'node:assert/strict'
import test from 'node:test'
import {
  initialMediaRecovery,
  mediaGrantExpiring,
  type MediaRecovery,
  recoverFromMediaError,
  retryMedia,
  settledMediaRecovery,
} from './request-attachment-media-recovery'

type Grant = { expires_at_unix: number }
type Step =
  | { error: Grant; generation?: number; at: number }
  | { granted: Grant }
  | { retry: Grant | null; at: number }

const fresh = (label: string): Grant => Object.assign({ expires_at_unix: 300 }, { label })
const expiring = (label: string): Grant => Object.assign({ expires_at_unix: 20 }, { label })

function replay(start: Grant | null, steps: Step[]) {
  let grant = start
  let recovery: MediaRecovery = initialMediaRecovery
  let renewals = 0
  for (const step of steps) {
    if ('granted' in step) {
      grant = step.granted
      continue
    }
    const next = 'error' in step
      ? recoverFromMediaError(recovery, {
          generation: step.generation ?? settledMediaRecovery(recovery, grant).generation,
          grant: step.error,
          now: step.at * 1_000,
        })
      : retryMedia(recovery, step.retry, step.at * 1_000)
    recovery = next.recovery
    if (next.renew) renewals += 1
  }
  const settled = settledMediaRecovery(recovery, grant)
  return { failed: settled.failed, generation: settled.generation, renewals }
}

test('a grant is renewed only when it has expired or is within the renewal margin', () => {
  const grant = { expires_at_unix: 100 }
  assert.equal(mediaGrantExpiring(grant, 69_999), false)
  assert.equal(mediaGrantExpiring(grant, 70_000), true)
})

const cases: Array<[string, Grant | null, (grants: Record<string, Grant>) => Step[], ReturnType<typeof replay>]> = [
  ['an error on a valid grant reloads the same URL once without renewing', fresh('a'),
    ({ a }) => [{ error: a!, at: 0 }], { failed: false, generation: 1, renewals: 0 }],
  ['an error after the reload shows the failure instead of reloading again', fresh('a'),
    ({ a }) => [{ error: a!, at: 0 }, { error: a!, at: 1 }], { failed: true, generation: 1, renewals: 0 }],
  ['errors from elements rendered before the reload are ignored', fresh('a'),
    ({ a }) => [{ error: a!, at: 0 }, { error: a!, generation: 0, at: 0 }], { failed: false, generation: 1, renewals: 0 }],
  ['an expiring grant is renewed and reloads once an identical renewal arrives', expiring('a'),
    ({ a }) => [{ error: a!, at: 0 }, { error: a!, at: 0 }, { granted: { ...a! } }], { failed: false, generation: 1, renewals: 1 }],
  ['the renewed grant failing shows the failure', expiring('a'),
    ({ a, b }) => [{ error: a!, at: 0 }, { granted: b! }, { error: b!, at: 2 }], { failed: true, generation: 1, renewals: 1 }],
  ['a later error after the recovery interval recovers again', fresh('a'),
    ({ a }) => [{ error: a!, at: 0 }, { error: a!, at: 60 }], { failed: false, generation: 2, renewals: 0 }],
  ['a manual retry reloads a valid grant and its failure shows again', fresh('a'),
    ({ a }) => [{ error: a!, at: 0 }, { error: a!, at: 1 }, { retry: a!, at: 2 }, { error: a!, at: 3 }],
    { failed: true, generation: 2, renewals: 0 }],
  ['a manual retry renews an expired grant and stays hidden until it arrives', fresh('a'),
    ({ a }) => [{ error: a!, at: 0 }, { error: a!, at: 1 }, { retry: a!, at: 290 }],
    { failed: true, generation: 1, renewals: 1 }],
  ['a manual renewal reloads when its grant arrives', fresh('a'),
    ({ a, b }) => [{ error: a!, at: 0 }, { error: a!, at: 1 }, { retry: a!, at: 290 }, { granted: b! }],
    { failed: false, generation: 2, renewals: 1 }],
  ['a manual retry without a grant only reloads the grant', null,
    () => [{ retry: null, at: 0 }], { failed: false, generation: 0, renewals: 1 }],
]

for (const [name, start, steps, expected] of cases) {
  test(name, () => {
    const grants = { a: start ?? fresh('a'), b: fresh('b') }
    assert.deepEqual(replay(start === null ? null : grants.a, steps(grants)), expected)
  })
}
