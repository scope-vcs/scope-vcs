import assert from 'node:assert/strict'
import test from 'node:test'
import {
  eventKindLabel,
  requestCheckEvaluationNote,
  requestEventBody,
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
