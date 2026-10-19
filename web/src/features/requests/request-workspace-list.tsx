import type { RepoParams } from '@/api/types'
import type { RequestQueueItemResponse, RequestQueueSection } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { useHydrated } from '@/lib/use-hydrated'
import { useUnixClock } from '@/lib/use-unix-clock'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { Check, LoaderCircle, Undo2, UserRound, UserRoundMinus } from 'lucide-react'
import type { CSSProperties } from 'react'
import type { RequestAttentionCommand } from './request-attention-api'
import { RequestDiscussionActorAvatar } from './request-discussion-byline'
import { RequestRailMarker } from './request-workspace-rail-marker'
import { RequestSnoozeMenu } from './request-snooze-menu'
import {
  REQUEST_MERGE_STEP_LABELS,
  requestAgeLabel,
  requestAttentionHeat,
  requestAttentionLabel,
  requestHasNewActivity,
} from './request-workspace-model'
import { actorHandle } from './request-actor'

export type RequestWorkspaceListProps = {
  items: { item: RequestQueueItemResponse; section: RequestQueueSection }[]
  emptyLabel: string
  loading: boolean
  skeleton: boolean
  error: string | null
  hasMore: boolean
  onRetry: () => void
  onLoadMore: () => void
  onAction: (item: RequestQueueItemResponse, command: RequestAttentionCommand) => void
  params: RepoParams
  pendingId: string | null
  selectedId?: string
  rail?: boolean
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
  if (!items.length && skeleton) return <RequestWorkspaceListSkeleton rail={rowProps.rail} />
  const rows = items.map(({ item, section }, index) => (
    <RequestWorkspaceRow item={item} key={item.request.id} section={section} stacked={adjacentStackRows(items[index - 1], { item, section }) || adjacentStackRows({ item, section }, items[index + 1])} {...rowProps} />
  ))
  return (
    <div className="request-workspace-rows">
      {rows}
      {!items.length && !error && (
        <p className="py-2 pr-4 pl-(--request-workspace-row-inset) text-[13px] text-muted-foreground">{emptyLabel}</p>
      )}
      {error && (
        <div
          className="flex items-center gap-2 py-3 pr-4 pl-(--request-workspace-row-inset) text-xs text-danger-strong"
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
          aria-busy={loading}
          disabled={loading}
          onClick={onLoadMore}
          size="sm"
          type="button"
          variant="ghost"
        >
          {loading ? <LoaderCircle className="animate-spin" /> : null}
          Load more
        </Button>
      )}
    </div>
  )
}

export function RequestWorkspaceListSkeleton({ rail = false }: { rail?: boolean }) {
  return (
    <div className="request-workspace-rows request-workspace-rows--pending">
      {[0, 1, 2].map((index) => (
        <div className="request-workspace-row" data-rail={rail ? '' : undefined} key={index}>
          <div className="request-workspace-row-link">
            <BlockSkeleton className={cn('rounded-full', rail ? 'size-[30px]' : 'size-5')} />
            <div className="min-w-0">
              <TextSkeleton className="h-[18px]" length="long" size="meta" />
              <TextSkeleton className="mt-[3px] h-[15px]" length="short" size="meta" />
            </div>
          </div>
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
  rail,
  stacked,
}: Pick<RequestWorkspaceListProps, 'onAction' | 'params' | 'pendingId' | 'rail' | 'selectedId'> & {
  item: RequestQueueItemResponse
  section: RequestQueueSection
  stacked: boolean
}) {
  const hydrated = useHydrated()
  const { request, attention, author } = item
  const nowUnix = useUnixClock()
  const selected = selectedId === request.id
  const pending = pendingId === request.id
  const hot = attention.group === 'needs_you'
  const unread = requestHasNewActivity(item)
  const mergeStep = rail ? REQUEST_MERGE_STEP_LABELS[item.next_step] : undefined
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
      className={cn('request-workspace-row', selected && 'request-workspace-row--selected', stacked && 'request-workspace-row--stacked')}
      data-heat={hot ? requestAttentionHeat(item.attention_at_unix, nowUnix) : 0}
      data-rail={rail ? '' : undefined}
      data-request-id={request.id}
      style={{ '--row-actions': `${actionCount ? actionCount * 35 + 6 : 0}px` } as CSSProperties}
    >
      {request.stack_parent ? <div className="request-workspace-row-parent text-[10px] text-muted-foreground">
        {request.stack_parent.kind === 'available' ? <><span>Based on </span><Link className="underline-offset-4 hover:underline" params={{ ...params, requestId: request.stack_parent.request.id }} search={(current) => ({ view: current.view })} to="/$owner/$repo/requests/$requestId">{request.stack_parent.request.title}</Link></> : 'Unavailable dependency'}
      </div> : null}
      <Link
        aria-current={selected ? 'page' : undefined}
        className="request-workspace-row-link"
        params={{ ...params, requestId: request.id }}
        preload="intent"
        search={(current) => ({ view: current.view })}
        to="/$owner/$repo/requests/$requestId"
      >
        <span className="request-workspace-row-avatar">
          {rail ? (
            <RequestRailMarker handle={actorHandle(author)} nextStep={item.next_step} unread={unread} />
          ) : (
            <RequestDiscussionActorAvatar handle={actorHandle(author)} small />
          )}
        </span>
        <span className="min-w-0">
          <span
            className={cn(
              'block truncate text-[13px] leading-[1.35] tracking-[-0.01em]',
              hot || selected ? 'font-medium text-foreground' : 'text-muted-foreground',
              unread && 'font-semibold',
            )}
            title={request.title}
          >
            {request.title}
          </span>
          <span className="request-workspace-row-meta mt-[3px] flex items-center gap-2 text-xs leading-[1.4]">
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
                {mergeStep && <span className="sr-only">, {mergeStep}</span>}
              </span>
            </span>
            <time
              className="request-workspace-row-age ml-auto shrink-0 text-xs text-muted-foreground"
              dateTime={new Date(item.attention_at_unix * 1_000).toISOString()}
              suppressHydrationWarning
            >
              {requestAgeLabel(item.attention_at_unix, nowUnix, hydrated)}
            </time>
          </span>
        </span>
      </Link>
      {actionCount > 0 && (
        <fieldset className="request-workspace-row-actions absolute right-2 flex gap-[3px] border-0 p-0">
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
                  title={label}
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

function adjacentStackRows(left: RequestWorkspaceListProps['items'][number] | undefined, right: RequestWorkspaceListProps['items'][number] | undefined) {
  if (!left || !right || left.section !== right.section) return false
  const leftParent = left.item.request.stack_parent
  const rightParent = right.item.request.stack_parent
  const leftId = leftParent?.kind === 'available' ? leftParent.request.id : null
  const rightId = rightParent?.kind === 'available' ? rightParent.request.id : null
  return leftId === right.item.request.id || rightId === left.item.request.id || Boolean(leftId && leftId === rightId)
}
