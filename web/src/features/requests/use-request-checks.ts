import { useCallback, useState } from 'react'
import type { RequestChecksResponse } from '@/api/types.generated'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { reconcileRequestState, requestStateResource } from './request-state-resource'

export type RequestChecksController = {
  approve: () => Promise<void>
  approving: boolean
  checks: RequestChecksResponse
  error: string | null
}

export function useRequestChecks({
  approve,
  identity,
  checks,
}: {
  approve: (expectedHeadOid: string) => Promise<RequestChecksResponse>
  identity: string
  checks: RequestChecksResponse
}): RequestChecksController {
  const [approving, setApproving] = useState(false)
  const [approveError, setApproveError] = useState<string | null>(null)

  const shownHeadOid = checks.head_oid
  const runApproval = useCallback(async () => {
    if (!identity || !shownHeadOid) return
    const snapshot = requestStateResource.getSnapshot(identity)
    setApproving(true)
    setApproveError(null)
    try {
      const result = await approve(shownHeadOid)
      if (result.head_oid === shownHeadOid) reconcileRequestState(identity, snapshot, (state) => ({
        ...state,
        checks: result,
        detail: { request: { ...state.detail.request, mergeability: result.mergeability } },
        auto_merge: { ...state.auto_merge, can_enable: false },
      }))
      requestStateResource.invalidate(identity)
    } catch (cause) {
      requestStateResource.invalidate(identity)
      setApproveError(resourceErrorMessage(cause, 'The checks could not be started.'))
    } finally {
      setApproving(false)
    }
  }, [approve, identity, shownHeadOid])

  return {
    approve: runApproval,
    approving,
    checks,
    error: approveError,
  }
}
