import { loadOptionalResource } from '@/api/http'
import { parseRequestParams } from '@/api/request-inputs'
import { loadRequestStateForRequest } from '@/api/requests'
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
