import assert from 'node:assert/strict'
import test from 'node:test'
import { parseCreateRepoInviteInput, parseUpdateRepoMemberInput } from './repo-inputs'

test('owner invitation and member edits retain explicit visibility grants and revocations', () => {
  for (const can_change_file_visibility of [true, false]) {
    const permissions = { can_push: true, can_change_file_visibility }
    assert.deepEqual(parseCreateRepoInviteInput({ owner: 'owner', repo: 'demo', email: 'member@example.com', permissions }).permissions, permissions)
    assert.deepEqual(parseUpdateRepoMemberInput({ owner: 'owner', repo: 'demo', member_user_id: 'member', permissions }).permissions, permissions)
  }
})

test('member actions require explicit boolean grants', () => {
  assert.deepEqual(parseCreateRepoInviteInput({ owner: 'owner', repo: 'demo', email: 'member@example.com', permissions: { can_push: 'true', can_change_file_visibility: 'true' } }).permissions, { can_push: false, can_change_file_visibility: false })
})
