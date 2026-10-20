import assert from 'node:assert/strict'
import test from 'node:test'
import type {
  GitHubConnectionDetailsResponse,
  GitHubConnectionResponse,
  GitHubMainPublicationResponse,
} from '../../api/types.generated'
import { githubMainPublicationView } from './repo-github-main-publication-model'

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

test('the toggle turns on only for a viewer allowed to publish to a repository that may receive main', () => {
  assert.deepEqual(githubMainPublicationView(github({})), {
    enabled: false,
    canToggle: true,
    status: null,
    problem: null,
  })
  assert.equal(githubMainPublicationView(github({}, {}, false))?.canToggle, false)
  assert.equal(
    githubMainPublicationView(github({}, { public_on_github: true, public_confirmed: false }))?.canToggle,
    false,
  )
  assert.equal(
    githubMainPublicationView(github({ enabled: true }, {}, false))?.canToggle,
    true,
  )
})

test('the latest publication reads as sending, published, or refused with GitHub’s reason', () => {
  const latest = (state: 'queued' | 'running' | 'succeeded' | 'failed', error: string | null = null) =>
    githubMainPublicationView(
      github({ enabled: true, latest: { state, commit_oid: COMMIT, error, updated_at_unix: 1_767_225_600 } }),
    )
  assert.equal(latest('queued')?.status, 'Publishing main (abcdef0).')
  assert.deepEqual(latest('running', 'GitHub refused the push: timeout'), {
    enabled: true,
    canToggle: true,
    status: 'Publishing main (abcdef0).',
    problem: 'GitHub refused the push: timeout',
  })
  assert.equal(latest('succeeded')?.status, 'Published main (abcdef0) on Jan 01, 2026, 12:00 AM UTC.')
  assert.deepEqual(latest('failed', 'GitHub main has commits Scope does not have.'), {
    enabled: true,
    canToggle: true,
    status: 'Main (abcdef0) was not published.',
    problem: 'GitHub main has commits Scope does not have.',
  })
})
