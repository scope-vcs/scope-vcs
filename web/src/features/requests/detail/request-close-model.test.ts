import assert from 'node:assert/strict'
import test from 'node:test'
import type { LinkedRequestResponse } from '@/api/types.generated'
import {
  ANOTHER_REQUEST,
  NO_REQUEST,
  requestCloseCommand,
  requestCloseDraft,
  requestReference,
} from './request-close-model'

const linked = (id: string, status: LinkedRequestResponse['status']) =>
  ({ id, status }) as LinkedRequestResponse

test('closing as completed preselects the only merged request that resolves it', () => {
  const merged = linked('req_merged', 'Merged')
  const draft = linked('req_draft', 'Draft')
  assert.equal(requestCloseDraft([draft, merged]).fixedBy, 'req_merged')
  assert.equal(requestCloseDraft([draft]).fixedBy, NO_REQUEST)
  assert.equal(requestCloseDraft([merged, linked('req_other', 'Merged')]).fixedBy, NO_REQUEST)
})

test('a close names a request only when its reason takes one', () => {
  const resolvedBy = [linked('req_fix', 'Merged')]
  const draft = { ...requestCloseDraft(resolvedBy), reference: ' #142 ' }
  assert.equal(requestCloseCommand(draft, resolvedBy), null)
  assert.deepEqual(requestCloseCommand({ ...draft, reason: 'not_planned' }, resolvedBy), { reason: 'not_planned' })
  assert.deepEqual(requestCloseCommand({ ...draft, reason: 'duplicate' }, resolvedBy), {
    reason: 'duplicate', named_request: 'gh-142',
  })
  assert.equal(requestCloseCommand({ ...draft, reason: 'duplicate', reference: ' ' }, resolvedBy), null)
  assert.deepEqual(requestCloseCommand({ ...draft, reason: 'completed' }, resolvedBy), {
    reason: 'completed', named_request: 'req_fix',
  })
  assert.deepEqual(requestCloseCommand({ ...draft, reason: 'completed', fixedBy: NO_REQUEST }, resolvedBy), {
    reason: 'completed',
  })
  assert.deepEqual(requestCloseCommand({ ...draft, reason: 'completed', fixedBy: ANOTHER_REQUEST }, resolvedBy), {
    reason: 'completed', named_request: 'gh-142',
  })
  assert.deepEqual(requestCloseCommand({ ...draft, reason: 'completed', fixedBy: NO_REQUEST }, []), {
    reason: 'completed', named_request: 'gh-142',
  })
})

test('a request is named by its ID, handle, GitHub issue number, or page link', () => {
  for (const { text, reference } of [
    { text: 'req_abc', reference: 'req_abc' },
    { text: 'gh-142', reference: 'gh-142' },
    { text: '#142', reference: 'gh-142' },
    { text: 'https://scopevcs.com/owner/repo/requests/req_abc/changes?view=public', reference: 'req_abc' },
  ]) {
    assert.equal(requestReference(text), reference)
  }
})
