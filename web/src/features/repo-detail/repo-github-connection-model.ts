import type {
  GitHubConnectionResponse,
  GitHubDisconnectReasonResponse,
} from '../../api/types.generated'
import { formatUnixDateUtc } from '../../lib/date-format'

export type GitHubConnectionView =
  | { kind: 'unconfigured' }
  | { kind: 'not_connected' }
  | { kind: 'connected'; name: string; url: string; detail: string }
  | { kind: 'disconnected'; name: string; url: string; reason: string }

export function githubConnectionView(github: GitHubConnectionResponse): GitHubConnectionView {
  if (!github.configured) return { kind: 'unconfigured' }
  const connection = github.connection
  if (!connection) return { kind: 'not_connected' }
  const name = connection.github_full_name
  const url = connection.github_url
  if (connection.disconnected) {
    return { kind: 'disconnected', name, url, reason: disconnectReason(connection.disconnected.reason) }
  }
  const by = connection.connected_by ? ` by @${connection.connected_by.handle}` : ''
  return {
    kind: 'connected',
    name,
    url,
    detail: `Connected${by} on ${formatUnixDateUtc(connection.connected_at_unix)} UTC.`,
  }
}

function disconnectReason(reason: GitHubDisconnectReasonResponse) {
  switch (reason) {
    case 'app_uninstalled':
      return 'The Scope GitHub App was uninstalled from the GitHub account.'
    case 'installation_suspended':
      return 'The Scope GitHub App was suspended on the GitHub account.'
    case 'repository_removed':
      return 'The repository was removed from the Scope GitHub App installation.'
  }
}
