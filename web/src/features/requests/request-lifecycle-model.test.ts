import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestSummaryResponse } from '@/api/types.generated'
import {
  canMergeRequest,
  checksHoldRequestMerge,
  hasRequestAutoMergeActions,
  hasRequestLifecycleActions,
} from './request-lifecycle-model'

test('only a ready request merges, and checks hold the merge instead of hiding it', () => {
  assert.equal(canMergeRequest(request('Ready')), true)
  assert.equal(canMergeRequest(request('ChecksPending')), false)
  assert.equal(checksHoldRequestMerge(request('ChecksPending')), true)
  assert.equal(checksHoldRequestMerge(request('ChecksAwaitingApproval')), true)
  assert.equal(checksHoldRequestMerge(request('Ready')), false)
  // A viewer who cannot merge never sees the held merge.
  assert.equal(checksHoldRequestMerge(request('ChecksFailed', false)), false)
  assert.equal(hasRequestLifecycleActions(request('ChecksFailed')), true)
  assert.equal(hasRequestLifecycleActions(request('ChecksFailed', false)), false)
})

test('auto-merge actions require loaded status, an available action, or an open dialog', () => {
  assert.equal(hasRequestAutoMergeActions(null), false)
  assert.equal(hasRequestAutoMergeActions({ can_enable: false, intent: null }), false)
  assert.equal(hasRequestAutoMergeActions({ can_enable: true, intent: null }), true)
  assert.equal(hasRequestAutoMergeActions({ can_enable: false, intent: {} as never }), true)
  assert.equal(hasRequestAutoMergeActions(null, true), true)
})

function request(
  status: RequestSummaryResponse['mergeability']['status'],
  canMerge = true,
): RequestSummaryResponse {
  return {
    mergeability: { status, reason: 'a check did not succeed' },
    permissions: { can_close: false, can_merge: canMerge, can_submit: false },
  } as RequestSummaryResponse
}
