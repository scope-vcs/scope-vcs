import assert from 'node:assert/strict'
import test from 'node:test'
import {
  eventKindLabel,
  requestCheckEvaluationNote,
  requestChecksWorkflowWarning,
  requestEventBody,
  requestGitHubPushNote,
  requestPublicGitHubNote,
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

test('the approval note says where approving sends the checks', () => {
  const awaiting = (github_push: RequestChecksResponse['github_push']) =>
    ({ state: 'awaiting-approval', message: null, checks: [], github_push }) as unknown as RequestChecksResponse
  assert.equal(
    requestCheckEvaluationNote(awaiting(null)),
    'These checks wait for a maintainer to start them.',
  )
  assert.equal(
    requestCheckEvaluationNote(
      awaiting({ state: 'awaiting_approval', branch: 'scope/requests/req_1', error: null }),
    ),
    'These checks wait for a maintainer. Approving sends this revision to GitHub Actions.',
  )
  // Why the checks cannot pass comes first.
  const disconnected = 'This repository is no longer connected to GitHub.'
  assert.equal(
    requestCheckEvaluationNote({ ...awaiting(null), message: disconnected }),
    disconnected,
  )
  assert.equal(
    requestCheckEvaluationNote({ ...awaiting(null), state: 'started', message: disconnected }),
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

test('a private request checked in a public GitHub repository says it is public there', () => {
  const checks = (private_request_on_public_github: boolean) =>
    ({ private_request_on_public_github }) as RequestChecksResponse
  assert.equal(requestPublicGitHubNote(checks(false)), null)
  assert.equal(
    requestPublicGitHubNote(checks(true)),
    'This private request’s checks run in a public GitHub repository, so its changes are public on GitHub.',
  )
})

test('the push note follows the tested commit on its way to GitHub', () => {
  const push = (
    state: NonNullable<RequestChecksResponse['github_push']>['state'],
    error: string | null = null,
  ) => ({ state, branch: 'scope/requests/req_1', error })
  assert.equal(requestGitHubPushNote(null), null)
  assert.equal(requestGitHubPushNote(push('awaiting_approval')), null)
  assert.deepEqual(requestGitHubPushNote(push('sending')), {
    text: 'Sending this revision to GitHub.',
    failed: false,
  })
  assert.deepEqual(requestGitHubPushNote(push('sending', 'remote rejected')), {
    text: 'Sending to GitHub again. The last attempt failed: remote rejected',
    failed: false,
  })
  assert.deepEqual(requestGitHubPushNote(push('sent')), {
    text: 'Sent to GitHub as scope/requests/req_1.',
    failed: false,
  })
  assert.deepEqual(requestGitHubPushNote(push('failed', 'remote rejected')), {
    text: 'Sending to GitHub failed: remote rejected',
    failed: true,
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
