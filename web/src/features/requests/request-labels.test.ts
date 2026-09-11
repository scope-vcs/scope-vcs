import assert from 'node:assert/strict'
import test from 'node:test'
import { requestEventBody } from './request-labels'
import type { RequestEventResponse } from '@/api/types.generated'

test('activity describes submission', () => {
  assert.equal(
    requestEventBody(event('Submitted', {
      Submitted: { head_oid: 'a'.repeat(40) },
    })),
    'aaaaaaaaaaaa',
  )
})

function event(kind: RequestEventResponse['kind'], payload: RequestEventResponse['payload']) {
  return { kind, payload } as RequestEventResponse
}
