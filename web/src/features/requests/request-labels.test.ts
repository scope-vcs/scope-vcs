import assert from 'node:assert/strict'
import test from 'node:test'
import {
  eventKindLabel,
  requestCheckEvaluationNote,
  requestChecksWorkflowWarning,
  requestEventBody,
  requestPublicChecksNote,
} from './request-labels'
import type {
  RequestChecksResponse,
  RequestEventResponse,
} from '@/api/types.generated'

test('a head nobody evaluated says so instead of claiming it asks for no checks', () => {
  const checks = (state: RequestChecksResponse['state']) =>
    ({ state, message: null }) as RequestChecksResponse
  assert.equal(
    requestCheckEvaluationNote(checks(null)),
    'The checks for this commit have not been worked out yet.',
  )
  assert.equal(
    requestCheckEvaluationNote(checks('no-checks')),
    'This head asks for no checks.',
  )
})

test('checks awaiting approval wait for a maintainer, after any reason they cannot pass', () => {
  const awaiting = { state: 'awaiting-approval', message: null } as RequestChecksResponse
  assert.equal(
    requestCheckEvaluationNote(awaiting),
    'These checks wait for a maintainer to start them.',
  )
  const disconnected = 'This repository is no longer connected to GitHub.'
  assert.equal(
    requestCheckEvaluationNote({ ...awaiting, message: disconnected }),
    disconnected,
  )
  assert.equal(
    requestCheckEvaluationNote({ ...awaiting, state: 'started', message: disconnected }),
    disconnected,
  )
})

test('approving workflow changes warns only the maintainer who can approve', () => {
  const checks = (can_approve: boolean, changes_github_workflows: boolean) =>
    ({ can_approve, changes_github_workflows }) as RequestChecksResponse
  assert.equal(
    requestChecksWorkflowWarning(checks(true, true)),
    'This request changes GitHub workflow files. Approving runs them with your repository’s secrets.',
  )
  assert.equal(requestChecksWorkflowWarning(checks(false, true)), null)
  assert.equal(requestChecksWorkflowWarning(checks(true, false)), null)
})

test('a private request checked in a public repository says it is public', () => {
  const checks = (private_request_on_public_github: boolean) =>
    ({ private_request_on_public_github }) as RequestChecksResponse
  assert.equal(requestPublicChecksNote(checks(false)), null)
  assert.deepEqual(requestPublicChecksNote(checks(true)), {
    label: 'Checks run publicly',
    detail: 'This private request’s checks run in a public repository, so its changes are public.',
  })
})

test('activity describes submission', () => {
  assert.equal(
    requestEventBody(event('Submitted', {
      Submitted: { head_oid: 'a'.repeat(40) },
    })),
    'aaaaaaaaaaaa',
  )
})

test('activity describes auto-merge authorization and terminal outcomes', () => {
  assert.equal(
    requestEventBody(event('AutoMergeEnabled', {
      AutoMergeEnabled: {
        head_oid: 'a'.repeat(40),
        intent_id: 'intent',
        revision_id: 'revision',
      },
    })),
    'Will merge aaaaaaaaaaaa when checks pass.',
  )
  assert.equal(
    requestEventBody(event('AutoMergeStopped', {
      AutoMergeStopped: {
        head_oid: 'a'.repeat(40),
        intent_id: 'intent',
        reason: 'ChecksFailed',
        revision_id: 'revision',
      },
    })),
    'Auto-merge stopped for aaaaaaaaaaaa: checks failed.',
  )
  assert.equal(eventKindLabel('AutoMergeFulfilled'), 'Merged automatically')
})

function event(kind: RequestEventResponse['kind'], payload: RequestEventResponse['payload']) {
  return { kind, payload } as RequestEventResponse
}
