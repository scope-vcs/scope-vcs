import type { CreateRequestAttachmentMediaGrantResponse } from '@/api/types.generated'
import { createCachedResource } from '@/lib/cached-resource'

export const requestAttachmentMediaGrantResource = createCachedResource<CreateRequestAttachmentMediaGrantResponse>({
  maxEntries: 64,
})

const activeAccessScopes = new Map<string, string>()

export function activateRequestAttachmentMediaScope(accessScope: string) {
  const owner = accessScopeOwner(accessScope)
  if (!owner) return
  const previous = activeAccessScopes.get(owner)
  if (previous && previous !== accessScope) {
    requestAttachmentMediaGrantResource.removeMatching(
      (identity) => identity.startsWith(`${previous}\0`),
    )
  }
  activeAccessScopes.delete(owner)
  activeAccessScopes.set(owner, accessScope)
  if (activeAccessScopes.size > 64) {
    const oldest = activeAccessScopes.keys().next().value
    if (oldest) activeAccessScopes.delete(oldest)
  }
}

export function resetRequestAttachmentMediaGrants() {
  requestAttachmentMediaGrantResource.clear()
  activeAccessScopes.clear()
}

function accessScopeOwner(accessScope: string) {
  try {
    const value: unknown = JSON.parse(accessScope)
    if (!Array.isArray(value) || typeof value[0] !== 'string') return null
    const viewer = typeof value[1] === 'string' ? value[1] : 'anonymous'
    return JSON.stringify([value[0], viewer])
  } catch {
    return null
  }
}
