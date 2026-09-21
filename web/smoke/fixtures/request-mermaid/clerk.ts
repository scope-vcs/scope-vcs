import { useSyncExternalStore } from 'react'

let viewer = 'viewer'
const listeners = new Set<() => void>()
const subscribe = (notify: () => void) => {
  listeners.add(notify)
  return () => { listeners.delete(notify) }
}

export function setFixtureViewer(next: string) {
  viewer = next
  listeners.forEach((notify) => notify())
}

export function useAuth() {
  const userId = useSyncExternalStore(subscribe, () => viewer)
  return { isLoaded: true, userId }
}
