import { Link } from '@tanstack/react-router'
import { GitCommit } from 'lucide-react'
import type { RequestDiscussion } from './request-discussion-types'
import { shortOid } from './request-labels'

export function RequestDiscussionAnchor({
  anchor,
  params,
}: {
  anchor: NonNullable<RequestDiscussion['anchor']>
  params: { owner: string; repo: string; request_id: string }
}) {
  const label = requestDiscussionAnchorLabel(anchor)

  return (
    <Link
      aria-label={label}
      className="inline-grid size-6 shrink-0 place-items-center rounded text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      params={{
        owner: params.owner,
        repo: params.repo,
        requestId: params.request_id,
      }}
      search={{
        commit: anchor.commit_oid ?? undefined,
        path: anchor.path ?? undefined,
        revision: anchor.revision_id,
      }}
      title={label}
      to="/$owner/$repo/requests/$requestId/changes"
    >
      <GitCommit aria-hidden="true" className="size-3.5" />
    </Link>
  )
}

function requestDiscussionAnchorLabel(
  anchor: NonNullable<RequestDiscussion['anchor']>,
) {
  const commit = anchor.commit_oid
    ? ` at commit ${shortOid(anchor.commit_oid)}`
    : ''
  const path = anchor.path ? ` for ${anchor.path.replace(/^\/+/, '')}` : ''
  return `View revision ${anchor.revision_position} changes${path}${commit}`
}
