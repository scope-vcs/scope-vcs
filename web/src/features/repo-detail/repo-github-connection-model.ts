import type {
  GitHubConnectionResponse,
  GitHubDisconnectReasonResponse,
} from '../../api/types.generated'
import type { RepoViews } from '../../api/repo-views'
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

export type GitHubVisibilityView =
  | { kind: 'public' }
  | { kind: 'unconfirmed'; canConfirm: boolean }

export function githubVisibilityView(github: GitHubConnectionResponse): GitHubVisibilityView | null {
  const connection = github.connection
  if (!github.configured || !connection || connection.disconnected || !connection.public_on_github) {
    return null
  }
  return connection.public_confirmed
    ? { kind: 'public' }
    : { kind: 'unconfirmed', canConfirm: github.can_confirm_public }
}

export function githubWithheldRequestsText(views: Pick<RepoViews, 'anyone' | 'name'>) {
  return views.anyone ? `requests outside the ${views.name(views.anyone)} view` : 'requests'
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
