import type { RequestAutoMergeResponse } from '@/api/types.generated'
import {
  resourceErrorMessage,
  useCachedResource,
} from '@/lib/use-cached-resource'
import { useCallback, useState } from 'react'
import type {
  AuthorizeRequestAutoMergeInput,
  CancelRequestAutoMergeInput,
} from './request-auto-merge-api'
import { requestAutoMergeResource } from './request-auto-merge-resource'

type AuthorizeInput = Pick<
  AuthorizeRequestAutoMergeInput,
  'expected_head_oid' | 'expected_revision_id'
>
type CancelInput = Pick<CancelRequestAutoMergeInput, 'expected_intent_id'>

export type RequestAutoMergeController = {
  authorize: (input: AuthorizeInput) => Promise<boolean>
  cancel: (input: CancelInput) => Promise<boolean>
  error: string | null
  pending: 'authorize' | 'cancel' | null
  status: RequestAutoMergeResponse | null
}

export function useRequestAutoMerge({
  authorize,
  cancel,
  identity,
  load,
}: {
  authorize: (input: AuthorizeInput) => Promise<RequestAutoMergeResponse>
  cancel: (input: CancelInput) => Promise<RequestAutoMergeResponse>
  identity: string
  load: (signal: AbortSignal) => Promise<RequestAutoMergeResponse>
}): RequestAutoMergeController {
  const resource = useCachedResource({
    fallbackError: 'Auto-merge status is unavailable.',
    identity,
    load,
    resource: requestAutoMergeResource,
  })
  const [pending, setPending] = useState<'authorize' | 'cancel' | null>(null)
  const [mutationError, setMutationError] = useState<string | null>(null)

  const run = useCallback(async (
    action: 'authorize' | 'cancel',
    mutate: () => Promise<RequestAutoMergeResponse>,
  ) => {
    const generation = requestAutoMergeResource.invalidationGeneration(identity)
    setPending(action)
    setMutationError(null)
    try {
      requestAutoMergeResource.writeIfNotInvalidated(
        identity,
        generation,
        await mutate(),
      )
      return true
    } catch (cause) {
      setMutationError(resourceErrorMessage(
        cause,
        action === 'authorize'
          ? 'Auto-merge could not be enabled.'
          : 'Auto-merge could not be canceled.',
      ))
      return false
    } finally {
      setPending(null)
    }
  }, [identity])

  const runAuthorize = useCallback(
    (input: AuthorizeInput) => run('authorize', () => authorize(input)),
    [authorize, run],
  )
  const runCancel = useCallback(
    (input: CancelInput) => run('cancel', () => cancel(input)),
    [cancel, run],
  )

  return {
    authorize: runAuthorize,
    cancel: runCancel,
    error: mutationError ?? resource.error,
    pending,
    status: resource.value,
  }
}
