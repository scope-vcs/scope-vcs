import { RelativeTimestamp } from '@/components/timestamp'
import { shortOid } from '@/lib/short-oid'
import { Link } from '@tanstack/react-router'
import { ChevronRight, GitCommit } from 'lucide-react'
import { createContext, use } from 'react'
import type { RequestRevisionPush } from './request-timeline-items'

// The request page loads activity; the discussion below it shows the pushes.
const RequestRevisionPushesContext = createContext<readonly RequestRevisionPush[]>([])

export const RequestRevisionPushesProvider = RequestRevisionPushesContext.Provider

export function useRequestRevisionPushes() {
  return use(RequestRevisionPushesContext)
}

/** A push in the discussion. The whole row opens that revision's changes. */
export function RequestRevisionRow({
  params,
  push,
}: {
  params: { owner: string; repo: string; request_id: string }
  push: RequestRevisionPush
}) {
  return (
    <Link
      className="request-revision-row relative grid grid-cols-[2rem_minmax(0,1fr)_auto] items-center gap-x-3 px-5 py-2.5 text-xs text-muted-foreground before:pointer-events-none before:absolute before:inset-x-5 before:top-0 before:border-t before:border-border before:content-[''] first:before:hidden hover:bg-muted focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring lg:px-7 lg:before:inset-x-7"
      params={{ owner: params.owner, repo: params.repo, requestId: params.request_id }}
      search={{ revision: push.id }}
      to="/$owner/$repo/requests/$requestId/changes"
    >
      <span className="flex justify-center">
        <GitCommit aria-hidden="true" className="size-4" />
      </span>
      <span className="min-w-0 truncate">
        <span className="font-medium text-foreground">{push.actor.handle}</span>
        {' pushed '}
        <span className="font-medium text-foreground">Revision {push.position}</span>
        <span aria-hidden="true"> · </span>
        <span className="font-mono text-[11px]">{shortOid(push.oldHeadOid)} → {shortOid(push.newHeadOid)}</span>
        {push.note ? <><span aria-hidden="true"> · </span>{push.note}</> : null}
        <span aria-hidden="true"> · </span>
        <RelativeTimestamp value={push.createdAtUnix} />
      </span>
      <span className="flex items-center gap-1 font-medium text-foreground">
        <span className="hidden sm:inline">View changes</span>
        <ChevronRight aria-hidden="true" className="size-3.5" />
      </span>
    </Link>
  )
}
