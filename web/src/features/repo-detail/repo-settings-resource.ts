import type { GitHubConnectionResponse, RepositoryCollaborationResponse } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import { onViewerChange } from '../../lib/viewer-state'
import { applyCollaborationResult, type CollaborationResult } from './repo-collaboration-results'

/** The settings page's server data. `null` parts are not visible to the viewer. */
export type RepoSettingsData = {
  collaboration: RepositoryCollaborationResponse | null
  github: GitHubConnectionResponse | null
}

export const repoSettingsResource = createCachedResource<RepoSettingsData>({
  maxEntries: 24,
  maxWeight: 4 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function retainCollaborationResult(scope: string, result: CollaborationResult) {
  const current = repoSettingsResource.peek(scope)
  if (!current) return
  repoSettingsResource.write(scope, {
    ...current,
    collaboration: applyCollaborationResult(current.collaboration, result),
  })
  repoSettingsResource.invalidate(scope)
}

/** Keeps the connection a mutation returned, then confirms it with a refresh. */
export function retainGitHubConnection(scope: string, github: GitHubConnectionResponse) {
  const current = repoSettingsResource.peek(scope)
  if (!current) return
  repoSettingsResource.write(scope, { ...current, github })
  repoSettingsResource.invalidate(scope)
}

/**
 * The GitHub setup page connects a repository without knowing the viewer's
 * settings scope. Every retained snapshot of that repository is refreshed.
 */
export function invalidateRepoSettings(repoId: string) {
  repoSettingsResource.invalidateMatching((identity) => {
    try {
      const scope: unknown = JSON.parse(identity)
      return Array.isArray(scope) && scope[0] === repoId
    } catch {
      return false
    }
  })
}

const refreshedForExpiry = new Map<string, number>()
onViewerChange(() => refreshedForExpiry.clear())

/**
 * An invite expires by the clock: the server writes nothing, so no repository
 * event refreshes this snapshot. Refresh it when the earliest pending invite
 * passes its deadline, once per deadline so a fast client clock cannot loop.
 */
export function refreshWhenNextInviteExpires(
  scope: string,
  collaboration: RepositoryCollaborationResponse | null,
) {
  let next = Infinity
  for (const invite of collaboration?.invites ?? []) {
    if (invite.state === 'Pending') next = Math.min(next, invite.expires_at_unix)
  }
  if (next === Infinity) return
  if (refreshedForExpiry.get(scope) === next) return
  const timer = setTimeout(() => {
    refreshedForExpiry.set(scope, next)
    repoSettingsResource.invalidate(scope)
  }, Math.max(0, next * 1000 - Date.now()))
  return () => clearTimeout(timer)
}
