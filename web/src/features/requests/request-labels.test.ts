import assert from 'node:assert/strict'
import test from 'node:test'
import { eventKindLabel, requestEventBody } from './request-labels'
import type { RequestEventResponse } from '@/api/types.generated'

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
