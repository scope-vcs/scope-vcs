import type { RepositoryInviteResponse } from '../../api/types.generated'
import { formatUnixDateUtc } from '../../lib/date-format'

/**
 * The invitations worth showing: pending ones, and expired ones that nothing
 * newer has replaced. A member or a newer invite for the same email makes an
 * old invitation irrelevant.
 */
export function visibleInvitations(
  invites: readonly RepositoryInviteResponse[],
  memberEmails: readonly string[],
): RepositoryInviteResponse[] {
  const members = new Set(memberEmails.map((email) => email.toLowerCase()))
  const pendingEmails = new Set<string>()
  for (const invite of invites) {
    if (invite.state === 'Pending') pendingEmails.add(invite.invited_email.toLowerCase())
  }
  const newestExpired = new Map<string, RepositoryInviteResponse>()
  for (const invite of invites) {
    const email = invite.invited_email.toLowerCase()
    if (invite.state !== 'Expired' || members.has(email) || pendingEmails.has(email)) continue
    const current = newestExpired.get(email)
    if (!current || invite.expires_at_unix > current.expires_at_unix) {
      newestExpired.set(email, invite)
    }
  }
  return [
    ...invites.filter((invite) => invite.state === 'Pending'),
    ...newestExpired.values(),
  ].sort((left, right) => left.invited_email.localeCompare(right.invited_email))
}

/** Delivery and expiry in one line. "Sent" never claims the inbox. */
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
