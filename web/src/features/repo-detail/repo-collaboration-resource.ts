import type { RepoCollaboration } from '../../api/types'
import { createCachedResource } from '../../lib/cached-resource'
import { applyCollaborationResult, type CollaborationResult } from './repo-collaboration-results'

export const repoCollaborationResource = createCachedResource<{ collaboration: RepoCollaboration | null }>({
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
