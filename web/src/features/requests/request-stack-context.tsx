import type { RequestDetailResponse, RequestStackNodeResponse, RequestStackStatus } from '@/api/types.generated'
import { Badge, type BadgeVariant } from '@/components/ui/badge'
import { Popover } from '@/components/ui/popover'
import { shortOid } from '@/lib/short-oid'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import type { ReactNode } from 'react'

const STATUS: Record<Exclude<RequestStackStatus, 'main'>, { label: string; variant: BadgeVariant }> = {
  waiting_for_parent: { label: 'Waiting for parent', variant: 'neutral' },
  parent_extended: { label: 'Parent extended', variant: 'info' },
  parent_rewritten: { label: 'Parent rewritten', variant: 'warning' },
  parent_closed: { label: 'Parent closed', variant: 'danger' },
  ancestor_blocked: { label: 'Ancestor blocked', variant: 'warning' },
  integration_conflict: { label: 'Integration conflict', variant: 'danger' },
  needs_main_update: { label: 'Update from main required', variant: 'warning' },
}

type StackParams = { owner: string; repo: string }

export function RequestStackContext({ detail, params, childrenLoading = false, childrenError = null, onLoadChildren }: {
  detail: RequestDetailResponse
  params: StackParams
  childrenLoading?: boolean
  childrenError?: string | null
  onLoadChildren?: () => void
}) {
  const { request, stack_navigation: navigation } = detail
  const status = request.stack_status === 'main' ? null : STATUS[request.stack_status]
  const children = navigation.children.requests
  if (request.base.kind === 'main' && !status && !children.length && !navigation.children.next_cursor) return null
  const parent = navigation.parent?.kind === 'available' ? navigation.parent.request : null
  const pin = request.base.kind === 'request'
    ? `Pinned at ${shortOid(request.base.head_oid)}${parent ? ` · parent now at ${shortOid(parent.head_oid)}` : ''}`
    : null
  const reason = request.mergeability.status === 'Ready' ? null : request.mergeability.reason
  return (
    <section aria-label="Request dependency" className="min-w-0 border-b border-border px-5 py-2.5 text-xs sm:px-6 lg:px-8">
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1.5">
        <ol aria-label="Stack" className="flex min-w-0 flex-1 flex-wrap items-center gap-y-1.5">
          <StackStep first>
            <span className="font-mono text-muted-foreground">main</span>
          </StackStep>
          {navigation.ancestors_unavailable ? (
            <StackStep dashed dot="hidden">
              <span className="text-muted-foreground">Unavailable</span>
            </StackStep>
          ) : null}
          {navigation.ancestors.map((node) => (
            <StackStep dot="ancestor" key={node.id}>
              <StackLink node={node} params={params} />
            </StackStep>
          ))}
          <StackStep dashed={request.base.kind === 'request' && !parent} dot="current">
            <span aria-current="page" className="min-w-0 max-w-[18rem] truncate font-medium text-foreground" title={request.title}>{request.title}</span>
          </StackStep>
          {children.map((node, index) => (
            <StackStep dot="child" joined={index === 0} key={node.id}>
              <StackLink node={node} params={params} />
            </StackStep>
          ))}
          {navigation.children.next_cursor && onLoadChildren ? (
            <li className="ml-3 shrink-0">
              <button className="text-muted-foreground underline-offset-4 hover:text-foreground hover:underline disabled:opacity-50" disabled={childrenLoading} onClick={onLoadChildren} type="button">
                {childrenLoading ? 'Loading…' : childrenError ? 'Retry children' : 'More children'}
              </button>
            </li>
          ) : null}
        </ol>
        {status ? (
          <Popover
            align="auto"
            label={status.label}
            panel={() => (
              <div className="w-64 space-y-1.5 text-xs text-muted-foreground">
                {reason ? <p className="first-letter:uppercase">{reason}</p> : null}
                {pin ? <p className="font-mono">{pin}</p> : null}
              </div>
            )}
            trigger={(props) => (
              <button {...props} className="rounded-md focus-visible:outline-2 focus-visible:outline-ring" type="button">
                <Badge variant={status.variant}>{status.label}</Badge>
              </button>
            )}
          />
        ) : null}
      </div>
      {childrenError ? <p className="mt-1.5 text-danger-strong" role="alert">{childrenError}</p> : null}
    </section>
  )
}

const DOTS = {
  ancestor: 'bg-muted-foreground',
  current: 'size-2.5 border-2 border-foreground bg-background',
  child: 'border border-muted-foreground bg-background',
  hidden: 'border border-dashed border-muted-foreground bg-background',
}

function StackStep({ children, dot, dashed = false, first = false, joined = true }: {
  children: ReactNode
  dot?: keyof typeof DOTS
  dashed?: boolean
  first?: boolean
  joined?: boolean
}) {
  return (
    <li className={cn('flex min-w-0 max-w-full items-center', !joined && 'ml-3')}>
      {first || !joined ? null : <span aria-hidden="true" className={cn('mx-1.5 w-4 shrink-0 border-t border-border-strong', dashed && 'border-dashed')} />}
      {dot ? <span aria-hidden="true" className={cn('mr-1.5 size-2 shrink-0 rounded-full', DOTS[dot])} /> : null}
      {children}
    </li>
  )
}

function StackLink({ node, params }: { node: RequestStackNodeResponse; params: StackParams }) {
  return (
    <Link className="min-w-0 max-w-[14rem] truncate text-muted-foreground underline-offset-4 hover:text-foreground hover:underline" params={{ ...params, requestId: node.id }} search={(current) => ({ view: current.view })} title={node.title} to="/$owner/$repo/requests/$requestId">
      {node.title}
    </Link>
  )
}
