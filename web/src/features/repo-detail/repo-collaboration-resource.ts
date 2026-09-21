import type { RepositoryCollaborationResponse } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import { applyCollaborationResult, type CollaborationResult } from './repo-collaboration-results'

export const repoCollaborationResource = createCachedResource<{ collaboration: RepositoryCollaborationResponse | null }>({
  maxEntries: 24,
  maxWeight: 4 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function retainCollaborationResult(scope: string, result: CollaborationResult) {
  const current = repoCollaborationResource.peek(scope)
  if (!current) return
  repoCollaborationResource.write(scope, {
    collaboration: applyCollaborationResult(current.collaboration, result),
  })
  repoCollaborationResource.invalidate(scope)
}

const refreshedForExpiry = new Map<string, number>()

/**
 * An invite expires by the clock: the server writes nothing, so no repository
 * event refreshes this snapshot. Refresh it when the earliest pending invite
 * passes its deadline, once per deadline so a fast client clock cannot loop.
 */
export function refreshWhenNextInviteExpires(
  scope: string,
  collaboration: RepositoryCollaborationResponse | null,
) {
  const expiries = (collaboration?.invites ?? [])
    .filter((invite) => invite.state === 'Pending')
    .map((invite) => invite.expires_at_unix)
  if (expiries.length === 0) return
  const next = Math.min(...expiries)
  if (refreshedForExpiry.get(scope) === next) return
  const timer = setTimeout(() => {
    refreshedForExpiry.set(scope, next)
    repoCollaborationResource.invalidate(scope)
  }, Math.max(0, next * 1000 - Date.now()))
  return () => clearTimeout(timer)
}
