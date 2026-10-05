import { createContext, use } from 'react'
export const Viewer = createContext('viewer-one')
export function useAuth() { return { isLoaded: true, userId: use(Viewer) } }
export function UserButton() { return null }
