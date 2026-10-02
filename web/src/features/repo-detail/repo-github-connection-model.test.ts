import assert from 'node:assert/strict'
import test from 'node:test'
import type { GitHubConnectionDetailsResponse } from '../../api/types.generated'
import { githubConnectionView, githubVisibilityView } from './repo-github-connection-model'

const connection: GitHubConnectionDetailsResponse = {
  github_full_name: 'octo/checks',
  github_url: 'https://github.com/octo/checks',
  connected_by: { id: 'user_owner', handle: 'owner' },
  connected_at_unix: 1_767_225_600,
  disconnected: null,
  public_on_github: false,
  public_confirmed: true,
}

test('the CI section describes each connection state', () => {
  assert.deepEqual(githubConnectionView({ configured: false, connection: null, required_checks: [], can_confirm_public: true, setup_check: null, run_import_count: 50, run_import: null }), { kind: 'unconfigured' })
  // A stored link is not offered when the server can no longer use it.
  assert.deepEqual(githubConnectionView({ configured: false, connection, required_checks: [], can_confirm_public: true, setup_check: null, run_import_count: 50, run_import: null }), { kind: 'unconfigured' })
  assert.deepEqual(githubConnectionView({ configured: true, connection: null, required_checks: [], can_confirm_public: true, setup_check: null, run_import_count: 50, run_import: null }), { kind: 'not_connected' })
  assert.deepEqual(githubConnectionView({ configured: true, connection, required_checks: [], can_confirm_public: true, setup_check: null, run_import_count: 50, run_import: null }), {
    kind: 'connected',
    name: 'octo/checks',
    url: 'https://github.com/octo/checks',
    detail: 'Connected by @owner on Jan 01, 2026, 12:00 AM UTC.',
  })
  assert.equal(
    githubConnectionView({ configured: true, connection: { ...connection, connected_by: null }, required_checks: [], can_confirm_public: true, setup_check: null, run_import_count: 50, run_import: null }).kind,
    'connected',
  )
  assert.deepEqual(
    githubConnectionView({
      configured: true,
      connection: { ...connection, disconnected: { reason: 'repository_removed', at_unix: 1_767_312_000 } },
      required_checks: [],
      can_confirm_public: true,
      setup_check: null,
      run_import_count: 50,
      run_import: null,
    }),
    {
      kind: 'disconnected',
      name: 'octo/checks',
      url: 'https://github.com/octo/checks',
      reason: 'The repository was removed from the Scope GitHub App installation.',
    },
  )
})

test('a public GitHub repository is shown, and one that became public waits for a confirmation', () => {
  const github = (patch: Partial<GitHubConnectionDetailsResponse>, can_confirm_public = true) => ({
    configured: true,
    connection: { ...connection, ...patch },
    required_checks: [],
    can_confirm_public,
    setup_check: null,
    run_import_count: 50,
    run_import: null,
  })
  assert.equal(githubVisibilityView(github({})), null)
  assert.deepEqual(githubVisibilityView(github({ public_on_github: true })), { kind: 'public' })
  assert.deepEqual(
    githubVisibilityView(github({ public_on_github: true, public_confirmed: false }, false)),
    { kind: 'unconfirmed', canConfirm: false },
  )
  assert.equal(
    githubVisibilityView(github({
      public_on_github: true,
      disconnected: { reason: 'app_uninstalled', at_unix: 1 },
    })),
    null,
  )
})
