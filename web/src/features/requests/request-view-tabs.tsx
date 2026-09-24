import type { RepoParams } from '@/api/types'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { GitCommit, MessageSquare, SlidersHorizontal } from 'lucide-react'

export function RequestViewTabs({
  actionsRef,
  params,
  rail,
}: {
  /** The description's edit control and editor actions render here. */
  actionsRef: (element: HTMLElement | null) => void
  params: RepoParams & { requestId: string }
  rail: boolean
}) {
  const tabClass = 'inline-flex h-11 items-center gap-2 border-b-2 px-1 text-sm font-medium transition-colors'
  return (
    <nav aria-label="Request views" className="request-view-tabs flex flex-wrap gap-x-5 px-5 lg:gap-x-6 lg:px-7">
      <Link
        activeOptions={{ exact: true }}
        activeProps={{ className: 'border-foreground text-foreground' }}
        className={tabClass}
        inactiveProps={{ className: 'border-transparent text-muted-foreground hover:text-foreground' }}
        params={params}
        preload="intent"
        resetScroll={false}
        search={{}}
        to="/$owner/$repo/requests/$requestId"
      >
        <MessageSquare className="size-3.5" />
        Discussion
      </Link>
      <Link
        activeProps={{ className: 'border-foreground text-foreground' }}
        className={tabClass}
        inactiveProps={{ className: 'border-transparent text-muted-foreground hover:text-foreground' }}
        params={params}
        preload="intent"
        resetScroll={false}
        search={{}}
        to="/$owner/$repo/requests/$requestId/changes"
      >
        <GitCommit className="size-3.5" />
        Changes
      </Link>
      <Link
        activeProps={{ className: 'border-foreground text-foreground' }}
        className={cn(tabClass, rail && 'hidden')}
        inactiveProps={{ className: 'border-transparent text-muted-foreground hover:text-foreground' }}
        params={params}
        preload="intent"
        resetScroll={false}
        search={{}}
        to="/$owner/$repo/requests/$requestId/details"
      >
        <SlidersHorizontal className="size-3.5" />
        Details
      </Link>
      <div className="ml-auto flex items-center" ref={actionsRef} />
    </nav>
  )
}
