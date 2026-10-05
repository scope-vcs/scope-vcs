import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestChecksResponse, RequestSummaryResponse } from '@/api/types.generated'
import {
  canMergeRequest,
  checksHoldRequestMerge,
  hasRequestAutoMergeActions,
  hasRequestLifecycleActions,
  withCurrentMergeability,
} from './request-lifecycle-model'

test('only a ready request merges, and checks hold the merge from a viewer who can merge instead of hiding it', () => {
  assert.equal(canMergeRequest(request('Ready')), true)
  assert.equal(canMergeRequest(request('ChecksPending')), false)
  assert.equal(checksHoldRequestMerge(request('ChecksPending')), true)
  assert.equal(checksHoldRequestMerge(request('ChecksAwaitingApproval')), true)
  assert.equal(checksHoldRequestMerge(request('ChecksNotEvaluated')), true)
  assert.equal(checksHoldRequestMerge(request('Ready')), false)
  assert.equal(checksHoldRequestMerge(request('ChecksFailed', false)), false)
  assert.equal(hasRequestLifecycleActions(request('ChecksFailed')), true)
  assert.equal(hasRequestLifecycleActions(request('ChecksFailed', false)), false)
})

test('auto-merge actions require loaded status, an available action or unended authorization, or an open dialog', () => {
  assert.equal(hasRequestAutoMergeActions(null), false)
  assert.equal(hasRequestAutoMergeActions({ can_enable: false, intent: null }), false)
  assert.equal(hasRequestAutoMergeActions({ can_enable: true, intent: null }), true)
  assert.equal(hasRequestAutoMergeActions({ can_enable: false, intent: { status: 'Active' } as never }), true)
  assert.equal(hasRequestAutoMergeActions({ can_enable: false, intent: { status: 'Stopped' } as never }), false)
  assert.equal(hasRequestAutoMergeActions(null, true), true)
})

test('refreshed checks supply the mergeability of an open request on the same head, not of an older head, a closed request or a draft', () => {
  const summary = { ...request('ChecksPending'), head_oid: 'a', state: 'Open' } as RequestSummaryResponse
  const checks = (status: RequestSummaryResponse['mergeability']['status'], head = 'a') =>
    ({ mergeability: { status, request_head_oid: head } }) as RequestChecksResponse
  assert.equal(withCurrentMergeability(summary, checks('Ready')).mergeability.status, 'Ready')
  assert.equal(withCurrentMergeability(summary, null), summary)
  assert.equal(withCurrentMergeability(summary, checks('Ready', 'b')), summary)
  const merged = { ...summary, state: 'Merged' } as RequestSummaryResponse
  assert.equal(withCurrentMergeability(merged, checks('Ready')), merged)
  assert.equal(withCurrentMergeability(summary, checks('Draft')), summary)
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
