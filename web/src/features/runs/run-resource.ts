import { useCallback, useEffect, useState, useSyncExternalStore } from 'react'
import { resourceErrorMessage, useRetryOnReconnect } from '../../lib/use-cached-resource'
import type { CachedResourceStore } from '../../lib/cached-resource'

type RunResourceValue = { updatedAt?: number }
const RUN_FRESHNESS_MS = 30_000

export function runResourceNeedsRecovery<T extends object & RunResourceValue>(resource: CachedResourceStore<T>, key: string) {
  const snapshot = resource.getSnapshot(key)
  return snapshot.stale || snapshot.error !== null ||
    Date.now() - (snapshot.value?.updatedAt ?? 0) >= RUN_FRESHNESS_MS
}

export function ensureRunResource<T extends object & RunResourceValue>(resource: CachedResourceStore<T>, key: string, load: (signal: AbortSignal) => Promise<T>, refreshVersion?: string) {
  const snapshot = resource.getSnapshot(key)
  const needsRead = !snapshot.pending && runResourceNeedsRecovery(resource, key)
  if (needsRead) resource.invalidate(key)
  return resource.load(key, needsRead ? refreshVersion ?? snapshot.version ?? '0' : snapshot.version ?? '0', load)
}

// SSR hands off data only for the same viewer/access identity. Client navigation
// and reopening subscribe to this owner before deciding whether it needs a read.
export function useRunResource<T extends object & RunResourceValue>({ identity, initialValue, load, resource, refreshVersion }: {
  identity: string | null
  initialValue: T | null
  load: (signal: AbortSignal) => Promise<T>
  resource: CachedResourceStore<T>
  refreshVersion?: string
}) {
  useState(() => { if (identity && initialValue) resource.seed(identity, initialValue, '0') })
  const snapshot = useSyncExternalStore(
    useCallback((listener) => identity ? resource.subscribe(identity, listener) : () => {}, [identity, resource]),
    useCallback(() => identity ? resource.getSnapshot(identity) : resource.getServerSnapshot(), [identity, resource]),
    resource.getServerSnapshot,
  )
  useEffect(() => {
    if (identity) void ensureRunResource(resource, identity, load, refreshVersion).catch(() => {})
  }, [identity, load, resource, refreshVersion, snapshot.stale])
  useRetryOnReconnect({
    error: snapshot.error === null ? null : resourceErrorMessage(snapshot.error, 'Run data is unavailable.'),
    retry: useCallback(() => {
      if (!identity) return
      resource.invalidate(identity)
      void ensureRunResource(resource, identity, load, refreshVersion).catch(() => {})
    }, [identity, load, resource, refreshVersion]),
  })
  return { ...snapshot, value: snapshot.value ?? initialValue }
}
