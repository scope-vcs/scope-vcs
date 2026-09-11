import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoMember } from '../../api/types'
import { repoCollaborationResource, retainCollaborationResult } from './repo-collaboration-resource'

const member: RepoMember = { user_id: 'member', handle: 'member', email: 'member@example.com', created_at_unix: 1, updated_at_unix: 1, permissions: { can_push: false, can_change_file_visibility: false } }

test('settings reuse one scoped snapshot and write results fence older reads without hiding updated permissions', async () => {
  repoCollaborationResource.clear()
  let requests = 0
  const load = async () => { requests += 1; return { collaboration: { members: [member], invites: [] } } }
  await repoCollaborationResource.load('owner-scope', '', load)
  await repoCollaborationResource.load('owner-scope', '', load)
  assert.equal(requests, 1)
  let release: (value: Awaited<ReturnType<typeof load>>) => void = () => {}
  repoCollaborationResource.invalidate('owner-scope')
  const oldRead = repoCollaborationResource.ensure('owner-scope', '', () => new Promise((resolve) => { release = resolve }))
  await Promise.resolve()
  const saved = { ...member, permissions: { ...member.permissions, can_push: true } }
  retainCollaborationResult('owner-scope', { type: 'memberUpdated', member: saved })
  assert.equal(repoCollaborationResource.peek('owner-scope')?.collaboration?.members[0].permissions.can_push, true)
  assert.equal(repoCollaborationResource.getSnapshot('owner-scope').stale, true)
  release({ collaboration: { members: [member], invites: [] } })
  await oldRead
  assert.equal(repoCollaborationResource.peek('owner-scope')?.collaboration?.members[0].permissions.can_push, true)
  assert.equal(repoCollaborationResource.peek('other-viewer'), null)
})
