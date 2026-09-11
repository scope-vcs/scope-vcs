import type { RepoParams } from '@/api/types'
import type { RequestQueueItemResponse, RequestQueueSection } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { BlockSkeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { Check, LoaderCircle, RefreshCw, UserRound } from 'lucide-react'
import type { RequestAttentionCommand } from './request-attention-api'
import { RequestSnoozeMenu } from './request-snooze-menu'
import { requestAttentionLabel } from './request-workspace-model'

export type RequestWorkspaceListProps = {
  items: { item: RequestQueueItemResponse; section: RequestQueueSection }[]
  emptyLabel: string
  loading: boolean
  error: string | null
  hasMore: boolean
  onRetry: () => void
  onLoadMore: () => void
  onAction: (item: RequestQueueItemResponse, command: RequestAttentionCommand) => void
  params: RepoParams
  pendingId: string | null
  selectedId?: string
}

export function RequestWorkspaceList({
  items,
  emptyLabel,
  loading,
  error,
  hasMore,
  onRetry,
  onLoadMore,
  ...rowProps
}: RequestWorkspaceListProps) {
  if (!items.length && loading) return <RequestWorkspaceListSkeleton />
  return (
    <div className="flex flex-col gap-[3px]">
      {items.map(({ item, section }) => (
        <RequestWorkspaceRow item={item} key={item.request.id} section={section} {...rowProps} />
      ))}
      {!items.length && !error && (
        <p className="px-3 py-3.5 text-[11px] text-muted-foreground">{emptyLabel}</p>
      )}
      {error && (
        <div
          className="flex items-center gap-2 px-3 py-3 text-[11px] text-danger-strong"
          role="alert"
        >
          <p className="min-w-0 flex-1">{error}</p>
          <Button onClick={onRetry} size="sm" type="button" variant="ghost">
            Try again
          </Button>
        </div>
      )}
      {hasMore && (
        <Button
          className="mx-auto mt-2 mb-3"
          disabled={loading}
          onClick={onLoadMore}
          size="sm"
          type="button"
          variant="ghost"
        >
          {loading ? (
            <>
              <LoaderCircle className="animate-spin" />
              Loading…
            </>
          ) : (
            'Load more'
          )}
        </Button>
      )}
    </div>
  )
}

export function RequestWorkspaceListSkeleton() {
  return (
    <div aria-label="Loading requests" className="flex flex-col gap-[3px]">
      {[0, 1, 2].map((index) => (
        <div className="space-y-3 px-3 py-4" key={index}>
          <BlockSkeleton className="h-4 w-4/5" />
          <BlockSkeleton className="h-3 w-3/5" />
        </div>
      ))}
    </div>
  )
}

function RequestWorkspaceRow({
  item,
  section,
  onAction,
  params,
  pendingId,
  selectedId,
}: Pick<RequestWorkspaceListProps, 'onAction' | 'params' | 'pendingId' | 'selectedId'> & {
  item: RequestQueueItemResponse
  section: RequestQueueSection
}) {
  const { request, attention, author } = item
  const selected = selectedId === request.id
  const pending = pendingId === request.id
  const canSetAside = section === 'active' && attention.can_set_aside
  const actions = [
    { action: 'settle', label: 'Settle', icon: Check, visible: canSetAside },
    {
      action: 'claim',
      label: 'Claim',
      icon: UserRound,
      visible: section === 'unclaimed' && attention.can_claim,
    },
    {
      action: 'restore',
      label: 'Restore',
      icon: RefreshCw,
      visible: section === 'set_aside' && attention.can_restore,
    },
  ] as const
  const hasActions = actions.some(({ visible }) => visible)
  return (
    <article
      className={cn(
        'request-workspace-row relative rounded-md transition-colors',
        selected && 'request-workspace-row--selected',
      )}
    >
      <Link
        aria-current={selected ? 'page' : undefined}
        className="block rounded-[inherit] px-3 py-[11px] text-left focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring max-[700px]:py-[13px]"
        params={{ ...params, requestId: request.id }}
        preload="intent"
        search={{}}
        to="/$owner/$repo/requests/$requestId"
      >
        <span
          className={cn(
            'mb-[5px] block text-[13px] leading-[1.45] font-medium tracking-[-0.05px]',
            section === 'set_aside' && !selected && 'font-normal text-muted-foreground',
          )}
        >
          {request.title}
        </span>
        <span className="flex items-center gap-[7px] text-[11px] leading-[1.4] text-muted-foreground">
          <span className="flex min-w-0 items-center gap-1.5">
            <span
              aria-hidden="true"
              className="size-1.5 shrink-0 rounded-full border border-current"
            />
            <span className="truncate">{requestAttentionLabel(item)}</span>
          </span>
          <span className="ml-auto shrink-0 text-[10px] whitespace-nowrap">
            {new Date(item.attention_at_unix * 1000).toLocaleDateString(undefined, {
              month: 'short',
              day: 'numeric',
            })}
          </span>
        </span>
        <span
          className={cn(
            'mt-[5px] flex items-center gap-1.5 overflow-hidden text-[11px] whitespace-nowrap text-muted-foreground',
            hasActions && 'pr-[63px]',
          )}
        >
          <span className="max-w-[65%] shrink-0 truncate">{author.handle}</span>
          <span aria-hidden="true">·</span>
          <span className="truncate font-mono">#{request.id}</span>
        </span>
      </Link>
      {hasActions && (
        <fieldset className="request-workspace-row-actions absolute right-2 bottom-[7px] flex gap-[3px] border-0 p-0">
          <legend className="sr-only">Request {request.id} actions</legend>
          {canSetAside && (
            <RequestSnoozeMenu
              disabled={pending}
              onSnooze={(until_unix) => onAction(item, { action: 'snooze', until_unix })}
              requestId={request.id}
            />
          )}
          {actions.map(
            ({ action, label, icon: Icon, visible }) =>
              visible && (
                <button
                  aria-label={`${label} request ${request.id}`}
                  className="request-workspace-row-action"
                  disabled={pending}
                  key={action}
                  onClick={() => onAction(item, { action })}
                  title={`${label} request ${request.id}`}
                  type="button"
                >
                  <Icon aria-hidden="true" />
                </button>
              ),
          )}
        </fieldset>
      )}
    </article>
  )
}
