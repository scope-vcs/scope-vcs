import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepositoryInviteResponse } from '../../api/types.generated'
import { invitationDetail, visibleInvitations } from './repo-invite-model'

const permissions = { can_push: false, can_change_file_visibility: false }
const invite = (
  id: string,
  invited_email: string,
  state: RepositoryInviteResponse['state'],
  expires_at_unix: number,
  email: RepositoryInviteResponse['email'] = null,
): RepositoryInviteResponse => ({ id, invited_email, permissions, state, expires_at_unix, email })

test('the list keeps pending invitations and only the expired ones nothing has replaced', () => {
  const visible = visibleInvitations(
    [
      invite('pending', 'b@example.com', 'Pending', 900),
      invite('replaced', 'B@example.com', 'Expired', 100),
      invite('older', 'c@example.com', 'Expired', 100),
      invite('newer', 'c@example.com', 'Expired', 200),
      invite('joined', 'member@example.com', 'Expired', 100),
      invite('revoked', 'd@example.com', 'Revoked', 900),
      invite('accepted', 'e@example.com', 'Accepted', 900),
    ],
    ['Member@example.com'],
  )

  assert.deepEqual(visible.map((item) => item.id), ['pending', 'newer'])
})

test('a replacement hides the expired invitation for good, whatever happens to the replacement', () => {
  const expired = invite('expired', 'a@example.com', 'Expired', 100)
  for (const replacementState of ['Revoked', 'Accepted'] as const) {
    const visible = visibleInvitations(
      [expired, invite('replacement', 'a@example.com', replacementState, 200)],
      [],
    )
    assert.deepEqual(visible, [], replacementState)
  }
})

test('delivery wording never claims more than the provider accepted', () => {
  const sent = { state: 'sent', requested_at_unix: 1 } as const
  assert.match(invitationDetail(invite('a', 'a@example.com', 'Pending', 1_790_000_000, sent)), /^Email sent · Expires /)
  assert.match(
    invitationDetail(invite('a', 'a@example.com', 'Pending', 1_790_000_000, { ...sent, state: 'failed' })),
    /^Delivery failed · Invitation still valid/,
  )
  assert.match(invitationDetail(invite('a', 'a@example.com', 'Pending', 1_790_000_000)), /^Not emailed/)
  assert.equal(
    invitationDetail(invite('a', 'a@example.com', 'Expired', 1, sent)),
    'This invitation can no longer be accepted',
  )
})
