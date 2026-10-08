import { loadOptionalResource } from '@/api/http'
import { parseRequestParams } from '@/api/request-inputs'
import { loadRequestStateForRequest } from '@/api/requests'
import type { RequestParams } from '@/api/types'
import type { RequestStateValue } from '@/features/requests/request-state-resource'
import { auth } from '@clerk/tanstack-react-start/server'
import { createServerFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'

export const loadRequestState = createServerFn({ method: 'GET' })
  .validator(parseRequestParams)
  .handler(async ({ data }) => {
    const [{ userId }, state] = await Promise.all([
      auth(),
      loadOptionalResource(() => loadRequestStateForRequest(data, getRequest().signal)),
    ])
    return { state, viewerId: userId }
  })

export async function loadRequestStateForViewer(
  data: RequestParams,
  viewerId: string | null,
  signal: AbortSignal,
): Promise<RequestStateValue> {
  const loaded = await loadRequestState({ data, signal })
  if (loaded.viewerId !== viewerId) throw new Error('The account changed while loading this request.')
  return { state: loaded.state }
}
