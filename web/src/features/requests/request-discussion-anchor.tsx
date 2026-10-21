import { Link } from '@tanstack/react-router'
import { GitCommit } from 'lucide-react'
import { displayRouteFilePath } from '@/lib/route-file'
import type { RequestDiscussion } from './request-discussion-types'
import { shortOid } from '@/lib/short-oid'
import { lineRangeLabel } from './request-changes-line-discussions'

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
      className="inline-flex h-6 shrink-0 items-center gap-1 rounded px-1.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
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
      <span aria-hidden="true" className="text-xs">
        rev {anchor.revision_position}
        {anchor.lines ? ` · L${lineSpan(anchor.lines)}` : null}
        {anchor.latest?.kind === 'mapped' ? ` → L${lineSpan(anchor.latest)}` : null}
        {anchor.latest?.kind === 'outdated' ? ' · outdated' : null}
      </span>
    </Link>
  )
}

function requestDiscussionAnchorLabel(
  anchor: NonNullable<RequestDiscussion['anchor']>,
) {
  const commit = anchor.commit_oid
    ? ` at commit ${shortOid(anchor.commit_oid)}`
    : ''
  const path = anchor.path ? ` for ${displayRouteFilePath(anchor.path)}` : ''
  const lines = anchor.lines ? ` ${lineRangeLabel(anchor.lines)}` : ''
  const latest = anchor.latest?.kind === 'outdated'
    ? ', outdated on the latest revision'
    : anchor.latest?.kind === 'mapped' && anchor.lines
      ? `, now ${lineRangeLabel({ ...anchor.lines, start_line: anchor.latest.start_line, end_line: anchor.latest.end_line })} on the latest revision`
      : ''
  return `View revision ${anchor.revision_position} changes${path}${lines}${commit}${latest}`
}

function lineSpan(lines: { start_line: number; end_line: number }) {
  return lines.start_line === lines.end_line ? `${lines.start_line}` : `${lines.start_line}-${lines.end_line}`
}
