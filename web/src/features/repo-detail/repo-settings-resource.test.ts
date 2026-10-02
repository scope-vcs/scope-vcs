import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepositoryInviteResponse, RepositoryMemberResponse } from '../../api/types.generated'
import { invalidateRepoSettings, refreshWhenNextInviteExpires, repoSettingsResource, retainCollaborationResult, retainGitHubConnection } from './repo-settings-resource'

const member: RepositoryMemberResponse = { user_id: 'member', handle: 'member', email: 'member@example.com', created_at_unix: 1, updated_at_unix: 1, permissions: { can_push: false, can_change_file_visibility: false } }

test('settings reuse one scoped snapshot and write results fence older reads without hiding updated permissions', async () => {
  repoSettingsResource.clear()
  let requests = 0
  const load = async () => { requests += 1; return { collaboration: { members: [member], invites: [] }, github: null } }
  await repoSettingsResource.load('owner-scope', '', load)
  await repoSettingsResource.load('owner-scope', '', load)
  assert.equal(requests, 1)
  let release: (value: Awaited<ReturnType<typeof load>>) => void = () => {}
  repoSettingsResource.invalidate('owner-scope')
  const oldRead = repoSettingsResource.ensure('owner-scope', '', () => new Promise((resolve) => { release = resolve }))
  await Promise.resolve()
  const saved = { ...member, permissions: { ...member.permissions, can_push: true } }
  retainCollaborationResult('owner-scope', { type: 'memberUpdated', member: saved })
  assert.equal(repoSettingsResource.peek('owner-scope')?.collaboration?.members[0].permissions.can_push, true)
  assert.equal(repoSettingsResource.getSnapshot('owner-scope').stale, true)
  release({ collaboration: { members: [member], invites: [] }, github: null })
  await oldRead
  assert.equal(repoSettingsResource.peek('owner-scope')?.collaboration?.members[0].permissions.can_push, true)
  assert.equal(repoSettingsResource.peek('other-viewer'), null)
  retainCollaborationResult('owner-scope', { type: 'memberRemoved', member: saved })
  assert.deepEqual(repoSettingsResource.peek('owner-scope')?.collaboration?.members, [])
})

test('a retained snapshot refreshes once when its earliest pending invite expires', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'Date'], now: 1_000_000 })
  repoSettingsResource.clear()
  const invite = (id: string, state: RepositoryInviteResponse['state'], expires_at_unix: number): RepositoryInviteResponse =>
    ({ id, invited_email: `${id}@example.com`, permissions: member.permissions, state, expires_at_unix, email: null })
  const collaboration = { members: [], invites: [invite('later', 'Pending', 1_900), invite('soon', 'Pending', 1_060), invite('revoked', 'Revoked', 1_010)] }
  await repoSettingsResource.load('expiry-scope', '', async () => ({ collaboration, github: null }))

  assert.equal(refreshWhenNextInviteExpires('expiry-scope', { members: [], invites: [] }), undefined)
  refreshWhenNextInviteExpires('expiry-scope', collaboration)
  t.mock.timers.tick(59_999)
  assert.equal(repoSettingsResource.getSnapshot('expiry-scope').stale, false)
  t.mock.timers.tick(1)
  assert.equal(repoSettingsResource.getSnapshot('expiry-scope').stale, true)

  // A client clock ahead of the server would get the same pending invite back.
  assert.equal(refreshWhenNextInviteExpires('expiry-scope', collaboration), undefined)
})

test('a GitHub connection change keeps the rest of the settings and refreshes every viewer of that repository', async () => {
  repoSettingsResource.clear()
  const collaboration = { members: [member], invites: [] }
  const scope = (repoId: string, viewer: string) => JSON.stringify([repoId, viewer, { actor: 'Owner' }])
  for (const identity of [scope('owner/repo', 'a'), scope('owner/repo', 'b'), scope('owner/other', 'a')]) {
    await repoSettingsResource.load(identity, '', async () => ({ collaboration, github: { configured: true, connection: null, required_checks: [], can_confirm_public: true } }))
  }
  const github = {
    configured: true,
    connection: {
      github_full_name: 'octo/checks', github_url: 'https://github.com/octo/checks',
      connected_by: null, connected_at_unix: 1, disconnected: null,
      public_on_github: false, public_confirmed: true,
    },
    required_checks: ['ci / test'],
    can_confirm_public: true,
  }
  retainGitHubConnection(scope('owner/repo', 'a'), github)
  assert.deepEqual(repoSettingsResource.peek(scope('owner/repo', 'a')), { collaboration, github })

  invalidateRepoSettings('owner/repo')
  assert.equal(repoSettingsResource.getSnapshot(scope('owner/repo', 'b')).stale, true)
  assert.equal(repoSettingsResource.getSnapshot(scope('owner/other', 'a')).stale, false)
  // Stale snapshots stay visible until their refresh answers.
  assert.deepEqual(repoSettingsResource.peek(scope('owner/repo', 'b'))?.collaboration, collaboration)
})
