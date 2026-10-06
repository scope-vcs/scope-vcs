import { repoViews, type RepoViews } from '../../api/repo-views'
import type { RepoLiveState } from '@/api/types'
import type { RepoChangeEvent } from '@/api/types.generated'
import {
  createContext,
  type ReactNode,
  use,
  useEffect,
  useMemo,
} from 'react'
import type { SubscribeToRepoChanges } from './repo-live-refresh'

type RepoLayoutContextValue = {
  live: RepoLiveState
  subscribe: SubscribeToRepoChanges
  views: RepoViews
}

const RepoLayoutContext = createContext<RepoLayoutContextValue | null>(null)

export function RepoLayoutProvider({
  children,
  live,
  subscribe,
}: {
  children: ReactNode
  live: RepoLiveState
  subscribe: SubscribeToRepoChanges
}) {
  const views = useMemo(() => repoViews(live.repo.views), [live.repo.views])
  const value = useMemo(() => ({ live, subscribe, views }), [live, subscribe, views])
  return (
    <RepoLayoutContext.Provider value={value}>
      {children}
    </RepoLayoutContext.Provider>
  )
}

export function useRepoLayout() {
  return useRepoLayoutContext().live
}

export function useRepoViews() {
  return useRepoLayoutContext().views
}

export function useRepoChangeSubscription(
  listener: (event: RepoChangeEvent) => void,
) {
  const { subscribe } = useRepoLayoutContext()
  useEffect(() => subscribe(listener), [listener, subscribe])
}

function useRepoLayoutContext() {
  const context = use(RepoLayoutContext)
  if (!context) throw new Error('repository layout context is unavailable')
  return context
}
