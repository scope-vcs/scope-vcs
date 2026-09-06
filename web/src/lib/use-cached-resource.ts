import { useCallback, useEffect, useSyncExternalStore } from 'react'
import type { CachedResourceStore } from './cached-resource'

type CachedResourceState<T extends object> =
  | { error: null; identity: null; status: 'idle'; value: null }
  | { error: null; identity: string; status: 'loading'; value: null }
  | { error: string | null; identity: string; status: 'loaded'; value: T }
  | { error: string; identity: string; status: 'failed'; value: null }

export type CachedResource<T extends object> = CachedResourceState<T> & {
  retry: () => void
  refreshing: boolean
}

export function useCachedResource<T extends object>({
  enabled = true,
  fallbackError,
  identity,
  load,
  resource,
  version = '',
}: {
  enabled?: boolean
  fallbackError: string
  identity: string | null
  load: (signal: AbortSignal) => Promise<T>
  resource: CachedResourceStore<T>
  version?: string
}): CachedResource<T> {
  const subscribe = useCallback((listener: () => void) => identity
    ? resource.subscribe(identity, listener)
    : () => {}, [identity, resource])
  const read = useCallback(() => identity
    ? resource.getSnapshot(identity)
    : resource.getServerSnapshot(), [identity, resource])
  const snapshot = useSyncExternalStore(subscribe, read, resource.getServerSnapshot)

  useEffect(() => {
    if (enabled && identity) void resource.ensure(identity, version, load)
  }, [enabled, identity, load, resource, snapshot.stale, version])

  const retry = useCallback(() => {
    if (!identity) return
    resource.invalidate(identity)
    if (enabled) void resource.ensure(identity, version, load)
  }, [enabled, identity, load, resource, version])

  const error = snapshot.error !== null ? resourceErrorMessage(snapshot.error, fallbackError) : null
  const state: CachedResourceState<T> = !identity
    ? { error: null, identity: null, status: 'idle', value: null }
    : snapshot.value !== null
      ? { error, identity, status: 'loaded', value: snapshot.value }
      : error
        ? { error, identity, status: 'failed', value: null }
        : { error: null, identity, status: 'loading', value: null }
  return {
    ...state,
    retry,
    refreshing: enabled && identity !== null && (snapshot.pending || snapshot.stale || snapshot.version !== version),
  }
}

export function resourceErrorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message.trim() ? error.message : fallback
}
