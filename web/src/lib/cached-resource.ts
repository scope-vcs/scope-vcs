import { createBoundedCache, type BoundedCacheOptions } from './bounded-cache'

type ResourceSnapshot<T> = {
  value: T | null
  error: unknown
  version: string | null
  stale: boolean
  pending: boolean
}

type ResourceAttempt<T> = {
  controller: AbortController
  promise: Promise<T | null>
  version: string
}

const emptySnapshot: ResourceSnapshot<never> = {
  value: null, error: null, version: null, stale: true, pending: false,
}

export type CachedResourceStore<T extends object> = ReturnType<typeof createCachedResource<T>>

// Requests belong to the resource, so leaving a page does not discard work
// another visit or subscriber can reuse.
export function createCachedResource<T extends object>(options: BoundedCacheOptions<T>) {
  const entries = createBoundedCache<string, ResourceSnapshot<T>>({
    ...options,
    weightOf: (snapshot) => snapshot.value === null ? 0 : options.weightOf?.(snapshot.value) ?? 0,
  })
  const attempts = new Map<string, ResourceAttempt<T>>()
  const listeners = new Map<string, Set<() => void>>()
  const visible = new Map<string, ResourceSnapshot<T>>()

  const getSnapshot = (identity: string): ResourceSnapshot<T> => visible.get(identity) ?? entries.peek(identity) ?? emptySnapshot
  const publish = (identity: string, snapshot: ResourceSnapshot<T>) => {
    entries.set(identity, snapshot)
    // Oversized results may be rendered by an active view without retaining
    // them in the navigation cache after its last subscriber leaves.
    if (listeners.has(identity)) visible.set(identity, snapshot)
    for (const listener of listeners.get(identity) ?? []) listener()
  }
  const cancel = (identity: string) => {
    attempts.get(identity)?.controller.abort()
    attempts.delete(identity)
  }
  const remove = (identity: string) => {
    cancel(identity)
    entries.delete(identity)
    visible.delete(identity)
    for (const listener of listeners.get(identity) ?? []) listener()
  }
  const identities = () => new Set([...entries.keys(), ...attempts.keys(), ...visible.keys()])

  function invalidate(identity: string) {
    const current = getSnapshot(identity)
    cancel(identity)
    if (current !== emptySnapshot) publish(identity, { ...current, error: null, stale: true, pending: false })
  }

  function ensure(identity: string, version: string, load: (signal: AbortSignal) => Promise<T>) {
    const existing = attempts.get(identity)
    if (existing?.version === version) return existing.promise
    entries.get(identity)
    const snapshot = getSnapshot(identity)
    if (!snapshot.stale && snapshot.version === version) return Promise.resolve(snapshot.value)
    cancel(identity)
    const controller = new AbortController()
    const attempt: ResourceAttempt<T> = { controller, promise: Promise.resolve(null), version }
    attempts.set(identity, attempt)
    publish(identity, { ...snapshot, error: null, version, stale: false, pending: true })
    attempt.promise = Promise.resolve().then(() => load(controller.signal)).then(
      (value) => {
        if (attempts.get(identity) !== attempt) return null
        publish(identity, { value, error: null, version, stale: false, pending: false })
        return value
      },
      (error: unknown) => {
        if (attempts.get(identity) !== attempt) return null
        publish(identity, { ...getSnapshot(identity), error: error ?? {}, stale: false, pending: false })
        return null
      },
    ).finally(() => {
      if (attempts.get(identity) === attempt) attempts.delete(identity)
    })
    return attempt.promise
  }

  return {
    ensure,
    async load(identity: string, version: string, load: (signal: AbortSignal) => Promise<T>): Promise<T> {
      if (getSnapshot(identity).error !== null) invalidate(identity)
      const value = await ensure(identity, version, load)
      if (value !== null) return value
      const snapshot = getSnapshot(identity)
      throw snapshot.error ?? new Error('Resource is no longer available.')
    },
    getSnapshot,
    getServerSnapshot: (): ResourceSnapshot<T> => emptySnapshot,
    invalidate,
    invalidateAll() {
      for (const identity of identities()) invalidate(identity)
    },
    invalidateMatching(matches: (identity: string) => boolean) {
      for (const identity of identities()) if (matches(identity)) invalidate(identity)
    },
    removeMatching(matches: (identity: string) => boolean) {
      for (const identity of identities()) if (matches(identity)) remove(identity)
    },
    peek: (identity: string) => getSnapshot(identity).value,
    read: (identity: string) => entries.get(identity)?.value ?? null,
    seed(identity: string, value: T, version = '') {
      if (getSnapshot(identity).version !== null) return
      publish(identity, { value, version, error: null, stale: false, pending: false })
    },
    write(identity: string, value: T, version = '') {
      cancel(identity)
      publish(identity, { value, version, error: null, stale: false, pending: false })
    },
    subscribe(identity: string, listener: () => void) {
      let subscribers = listeners.get(identity)
      if (!subscribers) listeners.set(identity, subscribers = new Set())
      subscribers.add(listener)
      visible.set(identity, getSnapshot(identity))
      return () => {
        subscribers.delete(listener)
        if (subscribers.size === 0) {
          listeners.delete(identity)
          visible.delete(identity)
        }
      }
    },
    clear() {
      for (const identity of attempts.keys()) cancel(identity)
      entries.clear()
      visible.clear()
      for (const subscribers of listeners.values()) {
        for (const listener of subscribers) listener()
      }
    },
    stats: entries.stats,
  }
}
