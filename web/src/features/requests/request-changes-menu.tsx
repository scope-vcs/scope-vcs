import { PanelState } from '@/components/empty-state'
import { MENU_LIST_ROW_CLASS, MenuListPanel } from '@/components/menu-list-panel'
import { PendingSurface } from '@/components/pending-surface'
import { RelativeTimestamp } from '@/components/timestamp'
import { Button } from '@/components/ui/button'
import { Popover } from '@/components/ui/popover'
import { TextSkeleton } from '@/components/ui/skeleton'
import { shortOid } from '@/lib/short-oid'
import { useCachedResource } from '@/lib/use-cached-resource'
import { Link } from '@tanstack/react-router'
import { ChevronDown, GitCommit, Search } from 'lucide-react'
import { useMemo, useState } from 'react'
import { requestActivityResource } from './request-activity-resource'
import { REQUEST_ACTIVITY_PAGE_SIZE } from './request-discussion-api'
import type { RequestActivityPage } from './request-discussion-types'
import { requestRevisionPushes, searchRequestRevisionPushes } from './request-revision-pushes'

type RequestRouteParams = { owner: string; repo: string; requestId: string }

type ActivitySource = {
  /** Null when the viewer cannot read request activity. */
  identity: string | null
  load: (signal: AbortSignal) => Promise<RequestActivityPage>
  version: string
}

/**
 * Changes opens a searchable list of the request's pushes, like the History
 * menu on the Code page. Each one opens the changes screen at that revision.
 */
export function RequestChangesMenu({ activity, params }: { activity: ActivitySource; params: RequestRouteParams }) {
  if (!activity.identity) {
    return (
      <Button asChild size="sm" variant="secondary">
        <Link params={params} to="/$owner/$repo/requests/$requestId/changes">
          <GitCommit />
          Changes
        </Link>
      </Button>
    )
  }
  return (
    <Popover
      align="auto"
      className="w-[min(30rem,calc(100vw-2rem))] p-0"
      label="Request changes"
      panel={(close) => (
        <RequestChangesMenuPanel activity={activity} onNavigate={close} params={params} />
      )}
      trigger={(props) => (
        <Button size="sm" type="button" variant="secondary" {...props}>
          <GitCommit />
          Changes
          <ChevronDown />
        </Button>
      )}
    />
  )
}

function RequestChangesMenuPanel({
  activity: { identity, load, version },
  onNavigate,
  params,
}: {
  activity: ActivitySource
  onNavigate: () => void
  params: RequestRouteParams
}) {
  const [query, setQuery] = useState('')
  const resource = useCachedResource({
    fallbackError: 'Request pushes could not be loaded.',
    identity,
    load,
    resource: requestActivityResource,
    version,
  })
  const pushes = useMemo(() => requestRevisionPushes(resource.value?.events ?? []), [resource.value])
  const matches = useMemo(() => searchRequestRevisionPushes(pushes, query), [pushes, query])
  const latestId = pushes[0]?.id
  // Activity holds only the latest events, so older revisions stay reachable from the changes screen.
  const partial = resource.status === 'loaded' &&
    (pushes.length === 0 || resource.value.events.length >= REQUEST_ACTIVITY_PAGE_SIZE)

  return (
    <MenuListPanel
      controls={(
        <search className="relative flex min-w-0 flex-1 items-center">
          <Search aria-hidden="true" className="pointer-events-none absolute left-2 size-3.5 text-muted-foreground" />
          <input
            aria-label="Search revisions"
            autoComplete="off"
            autoFocus
            className="h-8 w-full min-w-0 rounded border border-border bg-background pr-2 pl-7 text-xs placeholder:text-muted-foreground focus-visible:outline-2 focus-visible:outline-ring"
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search by note, author, revision or commit"
            type="search"
            value={query}
          />
        </search>
      )}
    >
      {resource.status === 'failed' ? (
        <PanelState tone="error">
          <span>{resource.error}</span>
          <Button onClick={resource.retry} size="sm" variant="secondary">Retry</Button>
        </PanelState>
      ) : resource.status !== 'loaded' ? (
        <PendingSurface label="Loading request pushes" onRetry={resource.retry}>
          <div className="grid gap-4 p-3">
            <TextSkeleton length="long" />
            <TextSkeleton length="medium" />
            <TextSkeleton length="long" />
          </div>
        </PendingSurface>
      ) : matches.length === 0 ? (
        <p className="px-3 py-6 text-center text-xs text-muted-foreground">
          {pushes.length === 0 ? 'No pushes in recent activity.' : 'No revisions match your search.'}
        </p>
      ) : (
        <ul className="divide-y divide-border">
          {matches.map((push) => (
            <li key={push.id}>
              <Link
                className={MENU_LIST_ROW_CLASS}
                onClick={onNavigate}
                params={params}
                search={{ revision: push.id }}
                title={push.note ?? undefined}
                to="/$owner/$repo/requests/$requestId/changes"
              >
                <span className="min-w-0">
                  <span className="block truncate text-[13px] font-medium">{push.note ?? `Revision ${push.position}`}</span>
                  <span className="mt-0.5 flex min-w-0 items-center gap-1.5 truncate text-xs text-muted-foreground">
                    <span className="shrink-0 font-medium text-foreground/80">Revision {push.position}</span>
                    {push.id === latestId ? <span className="shrink-0">· latest</span> : null}
                    <span aria-hidden="true">·</span>
                    <span className="truncate">{push.actor.handle}</span>
                    <span aria-hidden="true">·</span>
                    <RelativeTimestamp value={push.createdAtUnix} />
                  </span>
                </span>
                <span className="hidden font-mono text-[11px] text-muted-foreground sm:inline">{shortOid(push.newHeadOid)}</span>
              </Link>
            </li>
          ))}
        </ul>
      )}
      {partial ? (
        <Link
          className="flex items-center justify-center gap-1 border-t border-border px-3 py-2.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
          onClick={onNavigate}
          params={params}
          to="/$owner/$repo/requests/$requestId/changes"
        >
          {pushes.length === 0 ? 'Open changes' : 'Older revisions are on the changes screen'}
        </Link>
      ) : null}
    </MenuListPanel>
  )
}
