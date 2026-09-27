import type { RepoParams } from '@/api/types'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { ArrowLeft, ChevronLeft, ChevronRight } from 'lucide-react'
import type { ReactNode } from 'react'
import type { UpdateSearch } from './update-search'

// The loading state draws the same row with both neighbors disabled.
export function UpdateNavigation({
  newer,
  older,
  params,
  search,
}: {
  newer: string | null
  older: string | null
  params: RepoParams
  search: UpdateSearch
}) {
  return (
    <nav aria-label="Update navigation" className="flex items-center justify-between gap-3 border-b border-border px-5 py-2 text-xs sm:px-6">
      <Link
        className="flex items-center gap-1.5 rounded py-1 text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
        params={params}
        to="/$owner/$repo"
      >
        <ArrowLeft aria-hidden="true" className="size-3.5" /> Code
      </Link>
      <span className="flex items-center gap-1">
        <NeighborLink entryId={older} params={params} search={search}>
          <ChevronLeft aria-hidden="true" className="size-3.5" /> Older
        </NeighborLink>
        <NeighborLink entryId={newer} params={params} search={search}>
          Newer <ChevronRight aria-hidden="true" className="size-3.5" />
        </NeighborLink>
      </span>
    </nav>
  )
}

function NeighborLink({
  children,
  entryId,
  params,
  search,
}: {
  children: ReactNode
  entryId: string | null
  params: RepoParams
  search: UpdateSearch
}) {
  const className = 'flex items-center gap-1 rounded-md border border-border px-2.5 py-1'
  if (!entryId) {
    return <span aria-disabled="true" className={cn(className, 'text-muted-foreground/60')}>{children}</span>
  }
  return (
    <Link
      className={cn(className, 'hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring')}
      params={{ ...params, entryId }}
      search={search}
      to="/$owner/$repo/updates/$entryId"
    >
      {children}
    </Link>
  )
}
