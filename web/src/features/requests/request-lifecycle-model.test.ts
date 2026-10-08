import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestSummaryResponse } from '@/api/types.generated'
import {
  canMergeRequest,
  checksHoldRequestMerge,
  hasRequestAutoMergeActions,
  hasRequestLifecycleActions,
  requestShowsInvitees,
  requestSubmitsForReview,
} from './request-lifecycle-model'
import { repoViews } from '../../api/repo-views'

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


function request(
  status: RequestSummaryResponse['mergeability']['status'],
  canMerge = true,
): RequestSummaryResponse {
  return {
    mergeability: { status, reason: 'a check did not succeed' },
    permissions: { can_close: false, can_merge: canMerge, can_submit: false },
  } as RequestSummaryResponse
}

test('a public contributor asks for review and a member of any view marks ready', () => {
  assert.equal(requestSubmitsForReview({ author_role: 'Public' }), true)
  assert.equal(requestSubmitsForReview({ author_role: 'Member' }), false)
  assert.equal(requestSubmitsForReview({ author_role: 'Owner' }), false)
})

test('invitees show only on requests in the anonymous view', () => {
  const views = repoViews([
    { id: 'public', name: 'Public', includes: [], readers: 'anyone' },
    { id: 'private', name: 'Private', includes: 'all', readers: 'assigned' },
    { id: 'agent', name: 'Agent', includes: ['public'], readers: 'assigned' },
  ])
  const inView = (view: string) => ({ view, invitees: [] })
  assert.equal(requestShowsInvitees(inView('public'), views), true)
  assert.equal(requestShowsInvitees(inView('agent'), views), false)
  assert.equal(requestShowsInvitees(inView('private'), views), false)
  const noAnyone = repoViews([{ id: 'private', name: 'Private', includes: 'all', readers: 'assigned' }])
  assert.equal(requestShowsInvitees(inView('private'), noAnyone), false)
})
