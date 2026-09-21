import { createContext, use } from 'react'

export const FixtureViewer = createContext('adam')
export function useAuth() {
  return { isLoaded: true, userId: use(FixtureViewer) }
}
export function UserButton() { return null }
