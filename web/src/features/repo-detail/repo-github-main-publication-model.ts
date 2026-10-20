import type { GitHubConnectionResponse } from '../../api/types.generated'
import { formatUnixDateUtc } from '../../lib/date-format'

type GitHubMainPublicationView = {
  enabled: boolean
  canToggle: boolean
  status: string | null
  problem: string | null
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
        : `Publishing main (${commit}).`
  return {
    enabled: publication.enabled,
    canToggle: publication.enabled || (github.can_publish_main && connection.public_confirmed),
    status,
    problem: latest?.error ?? null,
  }
}
