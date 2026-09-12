import type {
  RepositoryCollaborationResponse,
  RepositoryInviteResponse,
  RepositoryMemberResponse,
} from '../../api/types.generated'

export type CollaborationResult =
  | { type: 'memberUpdated'; member: RepositoryMemberResponse }
  | { type: 'memberRemoved'; member: RepositoryMemberResponse }
  | { type: 'inviteUpdated'; invite: RepositoryInviteResponse }

// Apply authoritative write responses to the current cached snapshot before its
// background refresh, so the next edit cannot reuse the old permission values.
export function applyCollaborationResult(
  current: RepositoryCollaborationResponse | null,
  result: CollaborationResult,
): RepositoryCollaborationResponse | null {
  if (!current) return current
  switch (result.type) {
    case 'memberUpdated':
      return { ...current, members: current.members.map((member) => member.user_id === result.member.user_id ? result.member : member) }
    case 'memberRemoved':
      return { ...current, members: current.members.filter((member) => member.user_id !== result.member.user_id) }
    case 'inviteUpdated':
      return { ...current, invites: [...current.invites.filter((invite) => invite.id !== result.invite.id), result.invite] }
  }
}
