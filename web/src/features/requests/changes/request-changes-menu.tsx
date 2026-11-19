import type { RequestDiffStatResponse } from '@/api/types.generated'
import { DiffStatSummary } from '@/components/diff-stat'
import { PanelState } from '@/components/empty-state'
import { MENU_LIST_ROW_CLASS, MenuListPanel } from '@/components/menu-list-panel'
import { PendingSurface } from '@/components/pending-surface'
import { RelativeTimestamp } from '@/components/timestamp'
import { Button } from '@/components/ui/button'
import { Popover } from '@/components/ui/popover'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { type DiffStat, diffStatFilesLabel, diffStatLabel, diffStatWithChanges } from '@/lib/diff-stat'
import { shortOid } from '@/lib/short-oid'
import { useCachedResource } from '@/lib/use-cached-resource'
import { Link } from '@tanstack/react-router'
import { ChevronDown, GitCommit, Search } from 'lucide-react'
import { type ReactElement, useMemo, useState } from 'react'
import { requestActivityResource } from '../detail/request-activity-resource'
import { REQUEST_ACTIVITY_PAGE_SIZE } from '../discussion/request-discussion-api'
import type { RequestActivityPage } from '../discussion/request-discussion-types'
import { requestRevisionPushes, searchRequestRevisionPushes } from './request-revision-pushes'
import { actorHandle } from '../detail/request-actor'

type RequestRouteParams = { owner: string; repo: string; requestId: string }

type ActivitySource = {
  identity: string | null
  load: (signal: AbortSignal) => Promise<RequestActivityPage>
  version: string
}

export function RequestChangesMenu({
  activity,
  diffStat,
  params,
}: {
  activity: ActivitySource
  diffStat: RequestDiffStatResponse | null
  params: RequestRouteParams
}) {
  const changes = diffStatWithChanges(diffStat)
  const name = changes ? `Changes: ${diffStatLabel(changes)}` : undefined
  const label = changes ? (
    <>
      <DiffStatSummary className="request-changes-stat-exact pointer-events-none font-normal" detail="lines" stat={changes} />
      <DiffStatSummary className="request-changes-stat-compact pointer-events-none font-normal" detail="compact" stat={changes} />
    </>
  ) : (
    <>
      <GitCommit />
      Changes
    </>
  )
  if (!activity.identity) {
    return (
      <ChangesHint changes={changes}>
        <Button asChild size="sm" variant="secondary">
          <Link aria-label={name} params={params} to="/$owner/$repo/requests/$requestId/changes">
            {label}
          </Link>
        </Button>
      </ChangesHint>
    )
  }
  return (
    <Popover
      align="auto"
      className="w-[min(30rem,calc(100vw-2rem))] p-0"
      label="Request changes"
      renderPanel={(close) => (
        <RequestChangesMenuPanel activity={activity} changes={changes} onNavigate={close} params={params} />
      )}
      renderTrigger={(props) => (
        <ChangesHint changes={changes}>
          <Button aria-label={name} size="sm" type="button" variant="secondary" {...props}>
            {label}
            <ChevronDown />
          </Button>
        </ChangesHint>
      )}
    />
  )
}

function ChangesHint({ changes, children }: { changes: DiffStat | null; children: ReactElement }) {
  if (!changes) return children
  return (
    <TooltipProvider delayDuration={0}>
      <Tooltip>
        <TooltipTrigger asChild>{children}</TooltipTrigger>
        <TooltipContent>Changes · {diffStatFilesLabel(changes.changed_files)}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}

function RequestChangesMenuPanel({
  activity: { identity, load, version },
  changes,
  onNavigate,
  params,
}: {
  activity: ActivitySource
  changes: DiffStat | null
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
      <Link
        className="flex items-center gap-2 border-b border-border px-3 py-2.5 text-xs font-medium hover:bg-muted"
        onClick={onNavigate}
        params={params}
        search={{}}
        to="/$owner/$repo/requests/$requestId/changes"
      >
        <GitCommit className="size-3.5" /> Current changes
        {changes ? <span className="ml-auto font-normal text-muted-foreground">{diffStatFilesLabel(changes.changed_files)}</span> : null}
      </Link>
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
                search={{ revision: push.id, comparison: 'push' }}
                title={push.note ?? undefined}
                to="/$owner/$repo/requests/$requestId/changes"
              >
                <span className="min-w-0">
                  <span className="block truncate text-[13px] font-medium">{push.note ?? `Revision ${push.position}`}</span>
                  <span className="mt-0.5 flex min-w-0 items-center gap-1.5 truncate text-xs text-muted-foreground">
                    <span className="shrink-0 font-medium text-foreground/80">Revision {push.position}</span>
                    {push.id === latestId ? <span className="shrink-0">· latest</span> : null}
                    <span aria-hidden="true">·</span>
                    <span className="truncate">{actorHandle(push.actor)}</span>
                    <span aria-hidden="true">·</span>
                    <RelativeTimestamp value={push.createdAtUnix} />
                  </span>
                </span>
                <span className="hidden font-mono text-xs text-muted-foreground sm:inline">{shortOid(push.newHeadOid)}</span>
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
