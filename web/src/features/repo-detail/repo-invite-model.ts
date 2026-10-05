import type { RepositoryInviteResponse } from '../../api/types.generated'
import { formatUnixDateUtc } from '../../lib/date-format'

export const notEmailedNotice =
  'Invitation created, but not emailed: you have reached today\'s email limit. Copy a link, or send the email later.'

export function visibleInvitations(
  invites: readonly RepositoryInviteResponse[],
  memberEmails: readonly string[],
): RepositoryInviteResponse[] {
  const members = new Set(memberEmails.map((email) => email.toLowerCase()))
  const newest = new Map<string, RepositoryInviteResponse>()
  for (const invite of invites) {
    const email = invite.invited_email.toLowerCase()
    const current = newest.get(email)
    if (!current || isCreatedAfter(invite, current)) newest.set(email, invite)
  }
  const visible: RepositoryInviteResponse[] = []
  for (const [email, invite] of newest) {
    if (invite.state === 'Pending' || (invite.state === 'Expired' && !members.has(email))) {
      visible.push(invite)
    }
  }
  return visible.sort((left, right) => left.invited_email.localeCompare(right.invited_email))
}

function isCreatedAfter(invite: RepositoryInviteResponse, other: RepositoryInviteResponse) {
  if (invite.expires_at_unix !== other.expires_at_unix) return invite.expires_at_unix > other.expires_at_unix
  return invite.state === 'Pending' && other.state !== 'Pending'
}

export function invitationDetail(invite: RepositoryInviteResponse): string {
  if (invite.state === 'Expired') return 'This invitation can no longer be accepted'
  const expiry = `Expires ${formatUnixDateUtc(invite.expires_at_unix)} UTC`
  switch (invite.email?.state) {
    case 'queued':
      return `Sending email · ${expiry}`
    case 'sent':
      return `Email sent · ${expiry}`
    case 'failed':
      return `Delivery failed · Invitation still valid · ${expiry}`
    default:
      return `Not emailed · ${expiry}`
  }
}

export function emailActionLabel(invite: RepositoryInviteResponse): string {
  switch (invite.email?.state) {
    case 'queued':
      return 'Sending…'
    case 'failed':
      return 'Retry email'
    case 'sent':
      return 'Resend'
    default:
      return 'Send email'
  }
}
