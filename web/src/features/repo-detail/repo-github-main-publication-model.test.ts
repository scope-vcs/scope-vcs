import assert from 'node:assert/strict'
import test from 'node:test'
import type {
  GitHubConnectionDetailsResponse,
  GitHubConnectionResponse,
  GitHubMainPublicationResponse,
} from '../../api/types.generated'
import {
  MAIN_PUBLICATION_NOT_ALLOWED,
  MAIN_PUBLICATION_UNCONFIRMED,
  githubMainPublicationView,
} from './repo-github-main-publication-model'

const COMMIT = 'abcdef0123456789abcdef0123456789abcdef01'

function github(
  publication: Partial<GitHubMainPublicationResponse>,
  connection: Partial<GitHubConnectionDetailsResponse> = {},
  can_publish_main = true,
): GitHubConnectionResponse {
  return {
    configured: true,
    connection: {
      github_full_name: 'octo/checks',
      github_url: 'https://github.com/octo/checks',
      connected_by: null,
      connected_at_unix: 1_767_225_600,
      disconnected: null,
      public_on_github: false,
      public_confirmed: true,
      main_publication: { enabled: false, latest: null, ...publication },
      ...connection,
    },
    required_checks: [],
    can_confirm_public: true,
    can_publish_main,
    setup_check: null,
    run_import_count: 50,
    run_import: null,
  }
}

test('only a connected repository offers main publication', () => {
  assert.equal(githubMainPublicationView({ ...github({}), configured: false }), null)
  assert.equal(githubMainPublicationView({ ...github({}), connection: null }), null)
  assert.equal(
    githubMainPublicationView(
      github({}, { disconnected: { reason: 'app_uninstalled', at_unix: 1_767_312_000 } }),
    ),
    null,
  )
})

test('a switch that cannot turn on says why: the viewer may not publish, or the repository is public and unconfirmed', () => {
  assert.deepEqual(githubMainPublicationView(github({})), {
    enabled: false,
    blockedReason: null,
    status: null,
    attempt: null,
  })
  const unconfirmed = { public_on_github: true, public_confirmed: false }
  assert.equal(githubMainPublicationView(github({}, {}, false))?.blockedReason, MAIN_PUBLICATION_NOT_ALLOWED)
  assert.equal(githubMainPublicationView(github({}, unconfirmed))?.blockedReason, MAIN_PUBLICATION_UNCONFIRMED)
  assert.equal(githubMainPublicationView(github({}, unconfirmed, false))?.blockedReason, MAIN_PUBLICATION_NOT_ALLOWED)
  assert.equal(githubMainPublicationView(github({ enabled: true }, unconfirmed, false))?.blockedReason, null)
})

test('a push that will retry shows its last error quietly, and only a push that gave up reads as an error', () => {
  const latest = (state: 'queued' | 'running' | 'succeeded' | 'failed', error: string | null = null) =>
    githubMainPublicationView(
      github({ enabled: true, latest: { state, commit_oid: COMMIT, error, updated_at_unix: 1_767_225_600 } }),
    )
  assert.deepEqual(latest('queued'), {
    enabled: true,
    blockedReason: null,
    status: 'Publishing main (abcdef0).',
    attempt: null,
  })
  for (const state of ['queued', 'running'] as const) {
    assert.deepEqual(latest(state, 'GitHub answered 502'), {
      enabled: true,
      blockedReason: null,
      status: 'Publishing main (abcdef0). Scope will try again.',
      attempt: { text: 'The last attempt failed: GitHub answered 502', failed: false },
    })
  }
  assert.equal(latest('succeeded')?.status, 'Published main (abcdef0) on Jan 01, 2026, 12:00 AM UTC.')
  assert.deepEqual(latest('failed', 'GitHub main has commits Scope does not have.'), {
    enabled: true,
    blockedReason: null,
    status: 'Main (abcdef0) was not published.',
    attempt: { text: 'GitHub main has commits Scope does not have.', failed: true },
  })
})
