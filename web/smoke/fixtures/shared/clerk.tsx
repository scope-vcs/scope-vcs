import { createContext, use, useSyncExternalStore } from 'react'

let viewer = 'viewer'
const listeners = new Set<() => void>()
const subscribe = (notify: () => void) => {
  listeners.add(notify)
  return () => { listeners.delete(notify) }
}
const currentViewer = () => viewer
const getToken = async () => null

export const FixtureViewer = createContext<string | null>(null)

export function setFixtureViewer(next: string) {
  viewer = next
  listeners.forEach((notify) => notify())
}

export function useAuth() {
  const storedViewer = useSyncExternalStore(subscribe, currentViewer, currentViewer)
  return { getToken, isLoaded: true, userId: use(FixtureViewer) ?? storedViewer }
}

export function UserButton() { return null }
