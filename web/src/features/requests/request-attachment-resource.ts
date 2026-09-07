import type {
  RequestAttachmentLimitsResponse,
  RequestAttachmentResponse,
} from '@/api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'

export type RequestAttachmentResourceValue = {
  attachments: RequestAttachmentResponse[]
  limits: RequestAttachmentLimitsResponse
}

export const requestAttachmentResource = createCachedResource<RequestAttachmentResourceValue>({
  maxEntries: 16,
  maxWeight: 8 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

const activeAccessScopes = new Map<string, string>()

export function activateRequestAttachmentResourceScope(accessScope: string) {
  const owner = accessScopeOwner(accessScope)
  if (!owner) return
  const previous = activeAccessScopes.get(owner)
  if (previous && previous !== accessScope) {
    requestAttachmentResource.removeMatching(
      (identity) => identity.startsWith(`${previous}\0`),
    )
  }
  activeAccessScopes.delete(owner)
  activeAccessScopes.set(owner, accessScope)
  if (activeAccessScopes.size > 16) {
    const oldest = activeAccessScopes.keys().next().value
    if (oldest) activeAccessScopes.delete(oldest)
  }
}

export function resetRequestAttachmentResources() {
  requestAttachmentResource.clear()
  activeAccessScopes.clear()
}

export function requestAttachmentResourceIdentity(
  accessScope: string,
  requestId: string,
) {
  return `${accessScope}\0${requestId}`
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
