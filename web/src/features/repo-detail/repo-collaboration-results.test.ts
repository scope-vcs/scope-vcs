import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoCollaboration, RepoMember } from '../../api/types'
import { applyCollaborationResult } from './repo-collaboration-results'

const member = (user_id: string): RepoMember => ({
  user_id, handle: user_id, email: `${user_id}@example.com`, created_at_unix: 1, updated_at_unix: 1,
  permissions: { can_push: false, can_change_file_visibility: false },
})

test('independent successful member writes preserve each other before refresh and subsequent toggles use saved permissions', () => {
  const alice = member('alice')
  const bob = member('bob')
  let collaboration: RepoCollaboration | null = { members: [alice, bob], invites: [] }
  collaboration = applyCollaborationResult(collaboration, { type: 'memberUpdated', member: { ...alice, permissions: { ...alice.permissions, can_push: true } } })
  collaboration = applyCollaborationResult(collaboration, { type: 'memberUpdated', member: { ...bob, permissions: { ...bob.permissions, can_change_file_visibility: true } } })
  assert.equal(collaboration?.members[0].permissions.can_push, true)
  assert.equal(collaboration?.members[1].permissions.can_change_file_visibility, true)
  const nextAlice = collaboration!.members[0]
  collaboration = applyCollaborationResult(collaboration, { type: 'memberUpdated', member: { ...nextAlice, permissions: { ...nextAlice.permissions, can_change_file_visibility: true } } })
  assert.deepEqual(collaboration?.members[0].permissions, { can_push: true, can_change_file_visibility: true })
  collaboration = applyCollaborationResult(collaboration, { type: 'memberRemoved', member: bob })
  assert.deepEqual(collaboration?.members.map((value) => value.user_id), ['alice'])
})
