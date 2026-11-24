import type { LinkedRequestResponse, RequestCloseReason } from '@/api/types.generated'

export const NO_REQUEST = 'none'
export const ANOTHER_REQUEST = 'another'

export type RequestCloseDraft = {
  reason: RequestCloseReason | null
  fixedBy: string
  reference: string
}

export type RequestCloseCommand = { reason: RequestCloseReason; named_request?: string }

export function requestCloseDraft(resolvedBy: readonly LinkedRequestResponse[]): RequestCloseDraft {
  const [merged, ...otherMerged] = resolvedBy.filter((request) => request.status === 'Merged')
  return { reason: null, fixedBy: merged && otherMerged.length === 0 ? merged.id : NO_REQUEST, reference: '' }
}

export function requestReference(text: string) {
  const reference = text.trim().replace(/^.*\/requests\/([^/?#]+).*$/, '$1')
  return /^#[1-9]\d*$/.test(reference) ? `gh-${reference.slice(1)}` : reference
}

export function requestCloseCommand(
  draft: RequestCloseDraft,
  resolvedBy: readonly LinkedRequestResponse[],
): RequestCloseCommand | null {
  const reference = requestReference(draft.reference)
  switch (draft.reason) {
    case null:
      return null
    case 'not_planned':
      return { reason: draft.reason }
    case 'duplicate':
      return reference ? { reason: draft.reason, named_request: reference } : null
    case 'completed': {
      if (resolvedBy.some((request) => request.id === draft.fixedBy)) {
        return { reason: draft.reason, named_request: draft.fixedBy }
      }
      if (resolvedBy.length > 0 && draft.fixedBy !== ANOTHER_REQUEST) return { reason: draft.reason }
      if (reference) return { reason: draft.reason, named_request: reference }
      return draft.fixedBy === ANOTHER_REQUEST ? null : { reason: draft.reason }
    }
  }
}
