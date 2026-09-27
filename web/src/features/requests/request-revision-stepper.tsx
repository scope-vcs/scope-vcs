import type { RequestRevisionListResponse } from '@/api/types.generated'
import { RelativeTimestamp } from '@/components/timestamp'
import { Popover } from '@/components/ui/popover'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { ChevronDown, ChevronLeft, ChevronRight } from 'lucide-react'
import type { ReactNode } from 'react'
import { actorHandle } from './request-actor'

type RequestRouteParams = { owner: string; repo: string; requestId: string }
type Revision = RequestRevisionListResponse['revisions'][number]

const STEP_CLASS = 'flex items-center gap-1 rounded-md border border-border px-2.5 py-1'

/** Older and Newer step one push; the menu jumps to any revision the API returned. */
export function RequestRevisionStepper({
  params,
  revisions,
  selectedRevisionId,
}: {
  params: RequestRouteParams
  revisions: RequestRevisionListResponse
  selectedRevisionId: string | null
}) {
  const list = revisions.revisions
  const index = list.findIndex(({ id }) => id === selectedRevisionId)
  const selected = index < 0 ? null : list[index]
  if (!selected) return null
  const older = index > 0 ? list[index - 1] : null
  const newer = list[index + 1] ?? null

  return (
    <span className="flex items-center gap-1">
      <StepLink params={params} revision={older}>
        <ChevronLeft aria-hidden="true" className="size-3.5" /> Older
      </StepLink>
      <Popover
        className="w-[min(20rem,calc(100vw-2rem))] p-0"
        label="Request revisions"
        panel={(close) => (
          <ul className="max-h-[min(24rem,60vh)] divide-y divide-border overflow-y-auto text-xs">
            {[...list].reverse().map((revision) => (
              <li key={revision.id}>
                <Link
                  aria-current={revision.id === selected.id ? 'true' : undefined}
                  className="grid gap-0.5 px-3 py-2 hover:bg-muted focus-visible:bg-muted focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring aria-[current]:bg-muted"
                  onClick={close}
                  params={params}
                  search={{ revision: revision.id }}
                  to="/$owner/$repo/requests/$requestId/changes"
                >
                  <span className="font-medium text-foreground">
                    Revision {revision.position}
                    {revision.id === list.at(-1)?.id ? <span className="font-normal text-muted-foreground"> · latest</span> : null}
                  </span>
                  <span className="text-muted-foreground">
                    {actorHandle(revision.actor)} · <RelativeTimestamp value={revision.created_at_unix} /> · {commitCount(revision)}
                  </span>
                </Link>
              </li>
            ))}
          </ul>
        )}
        trigger={(props) => (
          <button
            className={cn(STEP_CLASS, 'font-medium hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring aria-expanded:bg-muted')}
            type="button"
            {...props}
          >
            Revision {selected.position}
            <ChevronDown aria-hidden="true" className="size-3.5" />
          </button>
        )}
      />
      <StepLink params={params} revision={newer}>
        Newer <ChevronRight aria-hidden="true" className="size-3.5" />
      </StepLink>
    </span>
  )
}

export function commitCount(revision: Revision) {
  return `${revision.commits.length} ${revision.commits.length === 1 ? 'commit' : 'commits'}`
}

function StepLink({
  children,
  params,
  revision,
}: {
  children: ReactNode
  params: RequestRouteParams
  revision: Revision | null
}) {
  if (!revision) {
    return <span aria-disabled="true" className={cn(STEP_CLASS, 'text-muted-foreground/60')}>{children}</span>
  }
  return (
    <Link
      className={cn(STEP_CLASS, 'hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring')}
      params={params}
      search={{ revision: revision.id }}
      title={`Revision ${revision.position}`}
      to="/$owner/$repo/requests/$requestId/changes"
    >
      {children}
    </Link>
  )
}
