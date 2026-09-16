import { useCachedResource, useRetryOnReconnect } from '@/lib/use-cached-resource'
import { loadAccountSession } from '@/routes/-account-session-actions'
import {
  accountSessionIdentity,
  accountSessionResource,
  loadAccountSessionValue,
} from './account-session-resource'

const load = (signal: AbortSignal) => loadAccountSessionValue(
  (attempt) => loadAccountSession({ signal: attempt }),
  signal,
)

// The shared account session resource owns the read, the retained value and the
// reconnect retry; viewers subscribe to it instead of fetching for themselves.
export function useAccountSession(viewerId: string | null) {
  const session = useCachedResource({
    fallbackError: 'Your account session is unavailable.',
    identity: viewerId === null ? null : accountSessionIdentity(viewerId),
    load,
    resource: accountSessionResource,
  })
  useRetryOnReconnect(session)
  return session
}
