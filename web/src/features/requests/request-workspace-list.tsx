import type { RepoParams } from '@/api/types'
import type { RequestQueueItemResponse, RequestQueueSection } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { BlockSkeleton } from '@/components/ui/skeleton'
import { useHydrated } from '@/lib/use-hydrated'
import { useUnixClock } from '@/lib/use-unix-clock'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { Check, LoaderCircle, Undo2, UserRound, UserRoundMinus } from 'lucide-react'
import type { CSSProperties } from 'react'
import type { RequestAttentionCommand } from './request-attention-api'
import { RequestDiscussionActorAvatar } from './request-discussion-byline'
import { RequestSnoozeMenu } from './request-snooze-menu'
import {
  requestAgeLabel,
  requestAttentionGroup,
  requestAttentionHeat,
  requestAttentionLabel,
  requestHasNewActivity,
} from './request-workspace-model'

export type RequestWorkspaceListProps = {
  items: { item: RequestQueueItemResponse; section: RequestQueueSection }[]
  emptyLabel: string
  loading: boolean
  /** First load only; a background refresh never swaps rows for placeholders. */
  skeleton: boolean
  error: string | null
  hasMore: boolean
  onRetry: () => void
  onLoadMore: () => void
  onAction: (item: RequestQueueItemResponse, command: RequestAttentionCommand) => void
  maintainer: boolean
  params: RepoParams
  pendingId: string | null
  selectedId?: string
}

export function RequestWorkspaceList({
  items,
  emptyLabel,
  loading,
  skeleton,
  error,
  hasMore,
  onRetry,
  onLoadMore,
  ...rowProps
}: RequestWorkspaceListProps) {
  if (!items.length && skeleton) return <RequestWorkspaceListSkeleton />
  const rows = items.map(({ item, section }) => (
    <RequestWorkspaceRow item={item} key={item.request.id} section={section} {...rowProps} />
  ))
  return (
    <div className="request-workspace-rows">
      {rows}
      {!items.length && !error && (
        <p className="px-4 py-3 text-[11px] text-muted-foreground">{emptyLabel}</p>
      )}
      {error && (
        <div
          className="flex items-center gap-2 px-4 py-3 text-[11px] text-danger-strong"
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
    <div aria-label="Loading requests" className="request-workspace-rows">
      {[0, 1, 2].map((index) => (
        <div className="grid grid-cols-[20px_1fr] gap-x-2.5 px-4 py-3" key={index}>
          <BlockSkeleton className="size-5 rounded-full" />
          <div className="space-y-2">
            <BlockSkeleton className="h-3.5 w-4/5" />
            <BlockSkeleton className="h-3 w-2/5" />
          </div>
        </div>
      ))}
    </div>
  )
}

function RequestWorkspaceRow({
  item,
  section,
  maintainer,
  onAction,
  params,
  pendingId,
  selectedId,
}: Pick<RequestWorkspaceListProps, 'maintainer' | 'onAction' | 'params' | 'pendingId' | 'selectedId'> & {
  item: RequestQueueItemResponse
  section: RequestQueueSection
}) {
  const hydrated = useHydrated()
  const { request, attention, author } = item
  const nowUnix = useUnixClock()
  const selected = selectedId === request.id
  const pending = pendingId === request.id
  const group = requestAttentionGroup(section, attention.reason, maintainer)
  const hot = group === 'needs_you'
  const unread = requestHasNewActivity(item)
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
      icon: Undo2,
      visible: section === 'set_aside' && attention.can_restore,
    },
    { action: 'release', label: 'Release', icon: UserRoundMinus, visible: attention.can_release },
  ] as const
  const actionCount = actions.filter(({ visible }) => visible).length + Number(canSetAside)
  return (
    <article
      className={cn('request-workspace-row', selected && 'request-workspace-row--selected')}
      data-group={group}
      data-heat={hot ? requestAttentionHeat(item.attention_at_unix, nowUnix) : 0}
      data-request-id={request.id}
      style={{ '--row-actions': `${actionCount * 30 + 6}px` } as CSSProperties}
    >
      <Link
        aria-current={selected ? 'page' : undefined}
        className="request-workspace-row-link"
        params={{ ...params, requestId: request.id }}
        preload="intent"
        search={{}}
        to="/$owner/$repo/requests/$requestId"
      >
        <RequestDiscussionActorAvatar handle={author.handle} small />
        <span className="min-w-0">
          <span
            className={cn(
              'block text-[13px] leading-[1.35] tracking-[-0.01em]',
              hot || selected ? 'font-medium text-foreground' : 'text-muted-foreground',
              unread && 'font-semibold',
            )}
          >
            {request.title}
          </span>
          <span className="request-workspace-row-meta mt-[3px] flex items-center gap-2 text-[11px] leading-[1.4]">
            <span
              className={cn(
                'flex min-w-0 items-center gap-1.5 truncate',
                hot ? 'text-foreground' : 'text-muted-foreground',
              )}
            >
              <span
                aria-hidden="true"
                className={cn(
                  'size-1.5 shrink-0 rounded-full border border-current',
                  hot && 'bg-current',
                )}
              />
              <span className="truncate" suppressHydrationWarning>
                {requestAttentionLabel(item, hydrated)}
              </span>
            </span>
            <time
              className="ml-auto shrink-0 font-mono text-[10px] text-muted-foreground/80"
              dateTime={new Date(item.attention_at_unix * 1_000).toISOString()}
              suppressHydrationWarning
            >
              {requestAgeLabel(item.attention_at_unix, nowUnix, hydrated)}
            </time>
          </span>
        </span>
      </Link>
      {actionCount > 0 && (
        <fieldset className="request-workspace-row-actions absolute right-2 bottom-2 flex gap-[3px] border-0 p-0">
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
