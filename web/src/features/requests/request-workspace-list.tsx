import type { RepoParams } from '@/api/types'
import type { RequestAttentionReason, RequestQueueItemResponse, RequestQueueSection } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { DiffStatSummary } from '@/components/diff-stat'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { diffStatWithChanges } from '@/lib/diff-stat'
import { useHydrated } from '@/lib/use-hydrated'
import { useUnixClock } from '@/lib/use-unix-clock'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { Check, Layers, LoaderCircle, Undo2, UserRound, UserRoundMinus } from 'lucide-react'
import { useRef, useState, type CSSProperties } from 'react'
import type { RequestAttentionCommand } from './request-attention-api'
import { RequestStatusGlyph } from './request-status-glyph'
import { RequestSnoozeMenu } from './request-snooze-menu'
import {
  requestAgeLabel,
  requestAttentionStale,
  requestHasNewActivity,
  requestStackParentLabel,
  requestStatusLabel,
} from './request-workspace-model'
import { actorHandle } from './request-actor'
import {
  foldStackedRequestRows,
  requestStackEdge,
  stackRequestRows,
  type RequestStackEdge,
  type RequestStackPlacement,
  type VisibleStackedRequestRow,
} from './request-workspace-stack'
import './request-workspace-stack.css'

const REQUEST_STACK_FOLD_MS = 100
const REQUEST_STACK_DOTS = 5
const REVIEW_REASONS: ReadonlySet<RequestAttentionReason> = new Set(['invited', 'claimed', 'unclaimed', 'claimed_elsewhere'])

export type QueueRow = { item: RequestQueueItemResponse; section: RequestQueueSection }

export type RequestWorkspaceListProps = {
  items: QueueRow[]
  emptyLabel: string
  loading: boolean
  skeleton: boolean
  error: string | null
  hasMore: boolean
  onRetry: () => void
  onLoadMore: () => void
  onAction: (item: RequestQueueItemResponse, command: RequestAttentionCommand) => void
  onCustomSnooze: (item: RequestQueueItemResponse) => void
  params: RepoParams
  pendingId: string | null
  selectedId?: string
  rail?: boolean
  foldedStacks: ReadonlySet<string>
  onFoldStack: (requestId: string, folded: boolean) => void
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
  foldedStacks,
  onFoldStack,
  ...rowProps
}: RequestWorkspaceListProps) {
  const [folding, setFolding] = useState<ReadonlyMap<string, ReadonlySet<string>>>(new Map())
  const [revealed, setRevealed] = useState<ReadonlySet<string>>(new Set())
  const foldTimers = useRef(new Map<string, number>())
  if (!items.length && skeleton) return <RequestWorkspaceListSkeleton />

  function cancelFold(parent: string) {
    window.clearTimeout(foldTimers.current.get(parent))
    foldTimers.current.delete(parent)
    setFolding((current) => {
      const next = new Map(current)
      next.delete(parent)
      return next
    })
  }

  function toggleStack({ row, descendants }: VisibleStackedRequestRow<QueueRow>) {
    const parent = row.item.request.id
    const ids = new Set(descendants.map(({ item }) => item.request.id))
    if (folding.has(parent)) {
      cancelFold(parent)
      return
    }
    if (foldedStacks.has(parent)) {
      setRevealed(ids)
      onFoldStack(parent, false)
      return
    }
    setFolding((current) => new Map(current).set(parent, ids))
    foldTimers.current.set(parent, window.setTimeout(() => {
      onFoldStack(parent, true)
      cancelFold(parent)
    }, REQUEST_STACK_FOLD_MS))
  }

  const leavingIds = new Set([...folding.values()].flatMap((ids) => [...ids]))
  let revealOrder = 0
  const visible = foldStackedRequestRows(stackRequestRows(items), foldedStacks)
  const rows = visible.map((entry, index) => {
    const { item, section } = entry.row
    const id = item.request.id
    return (
      <RequestWorkspaceRow
        descendants={entry.descendants.map(({ item }) => item)}
        folded={entry.folded.map(({ item }) => item)}
        item={item}
        key={id}
        leaving={leavingIds.has(id)}
        onToggleStack={() => toggleStack(entry)}
        placement={entry.placement}
        revealOrder={revealed.has(id) ? revealOrder++ : undefined}
        section={section}
        stackEdge={requestStackEdge(visible, index)}
        stackOpen={!foldedStacks.has(id) && !folding.has(id)}
        {...rowProps}
      />
    )
  })
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

export function RequestWorkspaceListSkeleton() {
  return (
    <div className="request-workspace-rows request-workspace-rows--pending">
      {[0, 1, 2].map((index) => (
        <div className="request-workspace-row" key={index}>
          <BlockSkeleton className="request-workspace-row-mark size-4 rounded-full" />
          <div className="request-workspace-row-body">
            <TextSkeleton className="h-[18px]" length="long" size="meta" />
            <TextSkeleton className="mt-[3px] h-[15px]" length="short" size="meta" />
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
  onCustomSnooze,
  params,
  pendingId,
  selectedId,
  rail,
  placement,
  descendants,
  folded,
  leaving,
  onToggleStack,
  revealOrder,
  stackEdge,
  stackOpen,
}: Pick<RequestWorkspaceListProps, 'onAction' | 'onCustomSnooze' | 'params' | 'pendingId' | 'rail' | 'selectedId'> & {
  item: RequestQueueItemResponse
  section: RequestQueueSection
  placement: RequestStackPlacement
  descendants: RequestQueueItemResponse[]
  folded: RequestQueueItemResponse[]
  leaving: boolean
  onToggleStack: () => void
  revealOrder: number | undefined
  stackEdge: RequestStackEdge | undefined
  stackOpen: boolean
}) {
  const hydrated = useHydrated()
  const { request, attention, author } = item
  const nowUnix = useUnixClock()
  const selected = selectedId === request.id
  const pending = pendingId === request.id
  const hot = attention.group === 'needs_you'
  const unread = requestHasNewActivity(item)
  const status = requestStatusLabel(item, hydrated)
  const handle = REVIEW_REASONS.has(attention.reason) ? actorHandle(author) : null
  const stackParent = requestStackParentLabel(item)
  const diffStat = diffStatWithChanges(request.diff_stat)
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
  const link = {
    onClick: () => {
      if (attention.reason === 'snooze_expired') onAction(item, { action: 'acknowledge' })
    },
    params: { ...params, requestId: request.id },
    preload: 'intent',
    search: (current: { view?: string }) => ({ view: current.view }),
    to: '/$owner/$repo/requests/$requestId',
  } as const
  return (
    <article
      className={cn('request-workspace-row', selected && 'request-workspace-row--selected')}
      data-holds-selection={folded.some(({ request }) => request.id === selectedId) || undefined}
      data-leaving={leaving || undefined}
      data-rail={rail ? '' : undefined}
      data-request-id={request.id}
      data-revealed={revealOrder !== undefined || undefined}
      data-stack-edge={stackEdge}
      data-stack-folded={(descendants.length > 0 && !stackOpen) || undefined}
      style={
        {
          '--row-action-count': actionCount,
          '--stack-depth': placement.depth,
          '--reveal-order': revealOrder,
        } as CSSProperties
      }
    >
      <RequestStackLines placement={{ ...placement, children: placement.children && stackOpen }} />
      <Link
        {...link}
        aria-hidden="true"
        className="request-workspace-row-mark"
        data-folded-count={folded.length || undefined}
        tabIndex={-1}
      >
        <RequestStatusGlyph className="request-status-glyph" nextStep={item.next_step} />
        {unread && <span className="request-workspace-row-unread" />}
      </Link>
      <div className="request-workspace-row-body">
        <div className="request-workspace-row-heading">
          <Link
            {...link}
            aria-current={selected ? 'page' : undefined}
            className={cn(
              'request-workspace-row-link text-[13px] leading-[1.35] tracking-[-0.01em]',
              hot || selected ? 'font-medium text-foreground' : 'text-muted-foreground',
              unread && 'font-semibold',
            )}
          >
            <span className="request-workspace-row-title" title={request.title}>
              {request.title}
            </span>
            <span className="sr-only" suppressHydrationWarning>
              {[handle, status, stackParent].filter(Boolean).map((detail) => `, ${detail}`)}
            </span>
          </Link>
          <time
            className="request-workspace-row-age"
            data-stale={(hot && requestAttentionStale(item.attention_at_unix, nowUnix)) || undefined}
            dateTime={new Date(item.attention_at_unix * 1_000).toISOString()}
            suppressHydrationWarning
          >
            {requestAgeLabel(item.attention_at_unix, nowUnix, hydrated)}
          </time>
          {actionCount > 0 && (
            <fieldset className="request-workspace-row-actions">
              <legend className="sr-only">Request {request.id} actions</legend>
              {canSetAside && (
                <RequestSnoozeMenu
                  disabled={pending}
                  onCustomSnooze={() => onCustomSnooze(item)}
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
        </div>
        <div className="request-workspace-row-meta">
          {handle && <span aria-hidden="true">{handle}</span>}
          <span aria-hidden="true" className="request-workspace-row-status" suppressHydrationWarning>
            {status}
          </span>
          {descendants.length > 0 && (
            <button
              aria-expanded={stackOpen}
              aria-label={`${stackOpen ? 'Collapse' : 'Expand'} ${descendants.length} stacked requests`}
              className="request-workspace-stack-pill"
              onClick={onToggleStack}
              type="button"
            >
              <Layers aria-hidden="true" />
              <span aria-hidden="true">{descendants.length} stacked</span>
              <span aria-hidden="true" className="request-workspace-stack-dots">
                {descendants.slice(0, REQUEST_STACK_DOTS).map(({ request, next_step }) => (
                  <i data-step={next_step} key={request.id} />
                ))}
              </span>
            </button>
          )}
          {diffStat && (
            <DiffStatSummary className="request-workspace-row-diff-stat" detail="compact" stat={diffStat} />
          )}
        </div>
      </div>
    </article>
  )
}

function RequestStackLines({ placement: { depth, guides, elbow, children } }: { placement: RequestStackPlacement }) {
  if (!depth && !children) return null
  return (
    <span aria-hidden="true" className="request-workspace-stack">
      {guides.map(({ column, dashed }) => (
        <span data-dashed={dashed || undefined} data-line="pass" key={column} style={{ '--stack-column': column } as CSSProperties} />
      ))}
      {elbow && <span data-line={elbow} style={{ '--stack-column': elbow === 'chain' ? depth : depth - 1 } as CSSProperties} />}
      {children && <span data-line="down" style={{ '--stack-column': depth } as CSSProperties} />}
    </span>
  )
}
