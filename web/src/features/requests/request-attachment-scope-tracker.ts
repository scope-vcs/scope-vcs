import { repoResourceScopeOwner } from '../repo-detail/repo-resource-scope'

export function createRequestAttachmentScopeTracker({
  maxOwners,
  removePreviousScope,
}: {
  maxOwners: number
  removePreviousScope: (accessScope: string) => void
}) {
  const activeAccessScopes = new Map<string, string>()

  return {
    activate(accessScope: string) {
      const owner = repoResourceScopeOwner(accessScope)
      if (!owner) return
      const previous = activeAccessScopes.get(owner)
      if (previous && previous !== accessScope) removePreviousScope(previous)
      activeAccessScopes.delete(owner)
      activeAccessScopes.set(owner, accessScope)
      // Forgetting an owner bounds tracking state without deleting its cached data.
      if (activeAccessScopes.size > maxOwners) {
        const oldest = activeAccessScopes.keys().next().value
        if (oldest) activeAccessScopes.delete(oldest)
      }
    },
    reset() {
      activeAccessScopes.clear()
    },
  }
}
