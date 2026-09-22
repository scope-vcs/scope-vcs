import type { RepositoryInviteResponse } from '../../api/types.generated'
import { formatUnixDateUtc } from '../../lib/date-format'

/** Shown when an invitation was created while the owner was out of daily emails. */
export const notEmailedNotice =
  'Invitation created, but not emailed: you have reached today\'s email limit. Copy a link, or send the email later.'

/**
 * The invitations worth showing: for each email, its newest invitation, if
 * that one is pending or expired. A newer invitation in any state, even a
 * revoked or accepted one, replaces the older ones, and a current member
 * needs no "send a new invitation" offer.
 */
export function visibleInvitations(
  invites: readonly RepositoryInviteResponse[],
  memberEmails: readonly string[],
): RepositoryInviteResponse[] {
  const members = new Set(memberEmails.map((email) => email.toLowerCase()))
  // An invitation expires seven days after it is created, so the latest
  // expiry is the newest invitation. Two created within the same second tie;
  // a pending one wins then, because only it can still be acted on.
  const newest = new Map<string, RepositoryInviteResponse>()
  for (const invite of invites) {
    const email = invite.invited_email.toLowerCase()
    const current = newest.get(email)
    if (
      !current ||
      invite.expires_at_unix > current.expires_at_unix ||
      (invite.expires_at_unix === current.expires_at_unix &&
        invite.state === 'Pending' &&
        current.state !== 'Pending')
    ) {
      newest.set(email, invite)
    }
  }
  const visible: RepositoryInviteResponse[] = []
  for (const [email, invite] of newest) {
    if (invite.state === 'Pending' || (invite.state === 'Expired' && !members.has(email))) {
      visible.push(invite)
    }
  }
  return visible.sort((left, right) => left.invited_email.localeCompare(right.invited_email))
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
