import { createContext, use, useMemo, useState, type ReactNode } from 'react'
import type { RepositorySearchFilter } from './repository-search-model'

export const REPOSITORY_SEARCH_INPUT_ID = 'repository-search-input'

type RepositorySearchContextValue = {
  filter: RepositorySearchFilter
  open: (filter?: RepositorySearchFilter) => void
  setFilter: (filter: RepositorySearchFilter) => void
}

const RepositorySearchContext = createContext<RepositorySearchContextValue | null>(null)

export function RepositorySearchProvider({ children }: { children: ReactNode }) {
  const [filter, setFilter] = useState<RepositorySearchFilter>('all')
  const value = useMemo(() => ({
    filter,
    open: (next: RepositorySearchFilter = 'all') => {
      setFilter(next)
      document.getElementById(REPOSITORY_SEARCH_INPUT_ID)?.focus()
    },
    setFilter,
  }), [filter])
  return <RepositorySearchContext value={value}>{children}</RepositorySearchContext>
}

export function useRepositorySearch() {
  const value = use(RepositorySearchContext)
  if (!value) throw new Error('Repository search is unavailable outside a repository.')
  return value
}
