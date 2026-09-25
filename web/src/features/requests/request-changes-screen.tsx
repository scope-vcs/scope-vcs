import type { RequestRevisionListResponse } from '@/api/types.generated'
import { Link } from '@tanstack/react-router'
import { ArrowLeft } from 'lucide-react'
import type { ReactNode } from 'react'
import { RequestRevisionStepper } from './request-revision-stepper'

/**
 * The changes screen replaces the request page rather than sitting under it:
 * a way back to the discussion, the request it belongs to, and the revision.
 * The loading state draws the same row without a stepper.
 */
export function RequestChangesScreen({
  children,
  params,
  revisions,
  selectedRevisionId,
  title,
}: {
  children: ReactNode
  params: { owner: string; repo: string; requestId: string }
  revisions: RequestRevisionListResponse | null
  selectedRevisionId: string | null
  title: ReactNode
}) {
  return (
    <div className="request-changes-screen w-full min-w-0">
      <nav
        aria-label="Request changes navigation"
        className="flex min-h-11 flex-wrap items-center gap-x-4 gap-y-2 border-b border-border px-5 py-2 text-xs sm:px-6 lg:px-8"
      >
        <Link
          className="flex shrink-0 items-center gap-1.5 rounded py-1 text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
          params={params}
          to="/$owner/$repo/requests/$requestId"
        >
          <ArrowLeft aria-hidden="true" className="size-3.5" /> Discussion
        </Link>
        <span className="min-w-0 flex-1 truncate text-muted-foreground">{title}</span>
        {revisions ? (
          <RequestRevisionStepper
            params={params}
            revisions={revisions}
            selectedRevisionId={selectedRevisionId}
          />
        ) : null}
      </nav>
      {children}
    </div>
  )
}
