import type { GitHubConnectionResponse } from '../../api/types.generated'
import { formatUnixDateUtc } from '../../lib/date-format'

export const MAIN_PUBLICATION_NOT_ALLOWED =
  'Only a maintainer who reads the full view and can change file visibility can turn this on.'
export const MAIN_PUBLICATION_UNCONFIRMED =
  'This GitHub repository is public and not confirmed. Allow it above before publishing main.'

type GitHubMainPublicationView = {
  enabled: boolean
  blockedReason: string | null
  status: string | null
  attempt: { text: string; failed: boolean } | null
}

export function githubMainPublicationView(
  github: GitHubConnectionResponse,
): GitHubMainPublicationView | null {
  const connection = github.connection
  if (!github.configured || !connection || connection.disconnected) return null
  const publication = connection.main_publication
  const latest = publication.latest
  const commit = latest?.commit_oid.slice(0, 7)
  const status = !latest
    ? null
    : latest.state === 'succeeded'
      ? `Published main (${commit}) on ${formatUnixDateUtc(latest.updated_at_unix)} UTC.`
      : latest.state === 'failed'
        ? `Main (${commit}) was not published.`
        : latest.error
          ? `Publishing main (${commit}). Scope will try again.`
          : `Publishing main (${commit}).`
  const attempt = !latest?.error
    ? null
    : latest.state === 'failed'
      ? { text: latest.error, failed: true }
      : { text: `The last attempt failed: ${latest.error}`, failed: false }
  const blockedReason = publication.enabled
    ? null
    : !github.can_publish_main
      ? MAIN_PUBLICATION_NOT_ALLOWED
      : !connection.public_confirmed
        ? MAIN_PUBLICATION_UNCONFIRMED
        : null
  return { enabled: publication.enabled, blockedReason, status, attempt }
}
