import type { RequestSummaryResponse } from '@/api/types.generated'
import { shortOid } from '@/lib/short-oid'
import { Link } from '@tanstack/react-router'

const STATUS_LABELS = {
  main: null,
  waiting_for_parent: 'Waiting for parent',
  parent_extended: 'Parent extended',
  parent_rewritten: 'Parent rewritten',
  parent_closed: 'Parent closed',
  ancestor_blocked: 'Ancestor blocked',
  integration_conflict: 'Integration conflict',
}

export function RequestStackContext({ request, params }: {
  request: RequestSummaryResponse
  params: { owner: string; repo: string }
}) {
  const label = STATUS_LABELS[request.stack_status]
  if (request.base.kind === 'main' && !label) return null
  return (
    <section aria-label="Request dependency" className="border-b border-border px-5 py-3 text-xs sm:px-6 lg:px-8">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        {request.base.kind === 'request' ? (
          <>
            <span className="text-muted-foreground">Based on</span>
            <Link
              className="break-all font-medium underline-offset-4 hover:underline"
              params={{ ...params, requestId: request.base.request_id }}
              to="/$owner/$repo/requests/$requestId"
            >
              Request {request.base.request_id}
            </Link>
            <span className="break-all font-mono text-muted-foreground" title={request.base.revision_id}>
              revision {request.base.revision_id} · {shortOid(request.base.head_oid)}
            </span>
          </>
        ) : <span className="text-muted-foreground">Based on main</span>}
        {label ? <span className="font-medium">· {label}</span> : null}
      </div>
      {request.stack_status === 'parent_extended' ? (
        <p className="mt-1.5 text-muted-foreground">The parent has new commits. This request’s pinned layer is unchanged and waits for the parent to land.</p>
      ) : null}
      {request.mergeability.status !== 'Ready' && request.mergeability.reason ? (
        <p className="mt-1.5 text-muted-foreground">{request.mergeability.reason}</p>
      ) : null}
    </section>
  )
}
