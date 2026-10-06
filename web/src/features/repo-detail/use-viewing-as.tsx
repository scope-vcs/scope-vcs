import type { ViewId } from '@/api/types.generated'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { useCallback } from 'react'
import { useRepoLayout, useRepoViews } from './repo-layout-context'
import { resolveViewingAs, viewingAsSearch } from './viewing-as'
import { ViewingAsPicker } from './viewing-as-picker'

export function useViewingAs() {
  const { repo } = useRepoLayout()
  const views = useRepoViews()
  const requested = useSearch({ strict: false, select: (search) => search.view })
  const navigate = useNavigate()
  const reader = repo.access.view
  const select = useCallback(
    (view: ViewId) => void navigate({
      resetScroll: false,
      search: (current) => ({ ...current, ...viewingAsSearch(view, reader) }),
      to: '.',
    }),
    [navigate, reader],
  )
  return {
    options: views.readableBy(reader),
    reader,
    select,
    view: resolveViewingAs(views, reader, requested),
  }
}

export function RepoViewingAsPicker({ className, compact }: { className?: string; compact?: boolean }) {
  const { options, select, view } = useViewingAs()
  return <ViewingAsPicker className={className} compact={compact} onChange={select} options={options} value={view} />
}
