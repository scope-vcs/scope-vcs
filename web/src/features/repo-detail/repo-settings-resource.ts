import type { GitHubConnectionResponse, RepositoryCollaborationResponse } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import { onViewerChange } from '../../lib/viewer-state'
import { applyCollaborationResult, type CollaborationResult } from './repo-collaboration-results'

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

export function retainGitHubConnection(scope: string, github: GitHubConnectionResponse) {
  const current = repoSettingsResource.peek(scope)
  if (!current) return
  repoSettingsResource.write(scope, { ...current, github })
  repoSettingsResource.invalidate(scope)
}

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
