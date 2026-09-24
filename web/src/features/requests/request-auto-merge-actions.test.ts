import assert from 'node:assert/strict'
import test from 'node:test'
import {
  autoMergeAuthorizer,
  autoMergeStopReasonText,
} from './request-auto-merge-model'

test('auto-merge status names the current authorizer without exposing their id', () => {
  assert.equal(
    autoMergeAuthorizer({ handle: 'adam', id: 'viewer' }, 'viewer'),
    'Authorized by you',
  )
  assert.equal(
    autoMergeAuthorizer({ handle: 'grace', id: 'other' }, 'viewer'),
    'Authorized by grace',
  )
})

test('auto-merge stop reasons read as plain text', () => {
  assert.equal(autoMergeStopReasonText('ChecksFailed'), 'checks failed')
  assert.equal(
    autoMergeStopReasonText('AccessRevoked'),
    'the authorizer no longer has access',
  )
})
