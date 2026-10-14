import type { RequestDetailResponse, RequestStackNodeResponse } from '@/api/types.generated'
import { shortOid } from '@/lib/short-oid'
import { Link, useNavigate } from '@tanstack/react-router'

const STATUS_LABELS = {
  main: null,
  waiting_for_parent: 'Waiting for parent',
  parent_extended: 'Parent extended',
  parent_rewritten: 'Parent rewritten',
  parent_closed: 'Parent closed',
  ancestor_blocked: 'Ancestor blocked',
  integration_conflict: 'Integration conflict',
  needs_main_update: 'Update from main required',
}

export function RequestStackContext({ detail, params, childrenLoading = false, childrenError = null, onLoadChildren }: {
  detail: RequestDetailResponse
  params: { owner: string; repo: string }
  childrenLoading?: boolean
  childrenError?: string | null
  onLoadChildren?: () => void
}) {
  const navigate = useNavigate()
  const { request, stack_navigation: navigation } = detail
  const label = STATUS_LABELS[request.stack_status]
  if (request.base.kind === 'main' && !label && !navigation.children.requests.length && !navigation.children.next_cursor) return null
  const parent = navigation.parent?.kind === 'available' ? navigation.parent.request : null
  const nodes = [...navigation.ancestors, ...navigation.children.requests]
  const go = (requestId: string) => {
    if (requestId) void navigate({ to: '/$owner/$repo/requests/$requestId', params: { ...params, requestId }, search: (current) => ({ view: current.view }) })
  }
  return (
    <section aria-label="Request dependency" className="min-w-0 border-b border-border px-5 py-3 text-xs sm:px-6 lg:px-8">
      <nav aria-label="Stack path" className="hidden flex-wrap items-center gap-x-2 gap-y-1 sm:flex">
        <span className="text-muted-foreground">Main</span>
        {navigation.ancestors_unavailable ? <span className="text-muted-foreground">/ Unavailable dependency</span> : null}
        {navigation.ancestors.map((node) => <span className="flex min-w-0 items-center gap-2" key={node.id}><span aria-hidden="true">/</span><StackLink node={node} params={params} /></span>)}
        <span aria-hidden="true">/</span><span className="min-w-0 break-words font-medium" aria-current="page">{request.title}</span>
      </nav>
      <label className="block sm:hidden">
        <span className="sr-only">Navigate stack</span>
        <select aria-label="Navigate stack" className="w-full min-w-0 rounded border border-border bg-background px-2 py-2" onChange={(event) => go(event.target.value)} value="">
          <option value="">{request.title} · depth {request.stack_depth}</option>
          {navigation.ancestors_unavailable ? <option disabled>Unavailable dependency</option> : null}
          {nodes.map((node) => <option key={node.id} value={node.id}>{node.stack_depth < request.stack_depth ? 'Ancestor' : 'Child'}: {node.title}</option>)}
        </select>
      </label>
      <div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-muted-foreground">
        <span>Depth {request.stack_depth}</span>
        {label ? <span className="font-medium text-foreground">· {label}</span> : null}
      </div>
      {request.base.kind === 'request' ? (
        <div className="mt-1.5 space-y-1 break-all font-mono text-muted-foreground">
          <p title={request.base.revision_id}>Pinned revision {request.base.revision_id} · {shortOid(request.base.head_oid)}</p>
          {parent ? <p title={parent.current_revision_id ?? undefined}>Current revision {parent.current_revision_id ?? 'unavailable'} · {shortOid(parent.head_oid)}</p> : <p>Unavailable dependency</p>}
        </div>
      ) : null}
      {navigation.children.requests.length ? <nav aria-label="Immediate children" className="mt-2 hidden flex-wrap items-center gap-x-3 gap-y-1 sm:flex">
        <span className="text-muted-foreground">Children</span>
        {navigation.children.requests.map((node) => <StackLink key={node.id} node={node} params={params} />)}
      </nav> : null}
      {navigation.children.next_cursor && onLoadChildren ? <button className="mt-2 underline underline-offset-4 disabled:opacity-50" disabled={childrenLoading} onClick={onLoadChildren} type="button">{childrenLoading ? 'Loading children…' : childrenError ? 'Retry children' : 'Load more children'}</button> : null}
      {childrenError ? <p className="mt-1.5 text-danger-strong" role="alert">{childrenError}</p> : null}
      {request.stack_status === 'parent_extended' ? <p className="mt-1.5 text-muted-foreground">The parent has new commits. This request’s pinned layer is unchanged and waits for the parent to land.</p> : null}
      {request.mergeability.status !== 'Ready' && request.mergeability.reason ? <p className="mt-1.5 text-muted-foreground">{request.mergeability.reason}</p> : null}
    </section>
  )
}

function StackLink({ node, params }: { node: RequestStackNodeResponse; params: { owner: string; repo: string } }) {
  return <Link className="min-w-0 break-words font-medium underline-offset-4 hover:underline" params={{ ...params, requestId: node.id }} search={(current) => ({ view: current.view })} to="/$owner/$repo/requests/$requestId">{node.title}</Link>
}
