import type { RepoParams } from '@/api/types'
import type { RequestQueueItemResponse, RequestQueueSection } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { useHydrated } from '@/lib/use-hydrated'
import { useUnixClock } from '@/lib/use-unix-clock'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { Check, ChevronDown, LoaderCircle, Undo2, UserRound, UserRoundMinus } from 'lucide-react'
import { useState, type CSSProperties } from 'react'
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
  requestStackParentLabel,
} from './request-workspace-model'
import { actorHandle } from './request-actor'
import {
  foldStackedRequestRows,
  stackRequestRows,
  type RequestStackPlacement,
  type VisibleStackedRequestRow,
} from './request-workspace-stack'
import './request-workspace-stack.css'

const REQUEST_STACK_FOLD_MS = 100

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
  const [folding, setFolding] = useState<{ parent: string; ids: ReadonlySet<string> } | null>(null)
  const [revealed, setRevealed] = useState<ReadonlySet<string>>(new Set())
  if (!items.length && skeleton) return <RequestWorkspaceListSkeleton rail={rowProps.rail} />

  function toggleStack({ row, descendants }: VisibleStackedRequestRow<QueueRow>) {
    const parent = row.item.request.id
    const ids = new Set(descendants.map(({ item }) => item.request.id))
    if (foldedStacks.has(parent)) {
      setRevealed(ids)
      onFoldStack(parent, false)
      return
    }
    setFolding({ parent, ids })
    window.setTimeout(() => {
      onFoldStack(parent, true)
      setFolding(null)
    }, REQUEST_STACK_FOLD_MS)
  }

  let revealOrder = 0
  const rows = foldStackedRequestRows(stackRequestRows(items), foldedStacks).map((entry) => {
    const { item, section } = entry.row
    const id = item.request.id
    return (
      <RequestWorkspaceRow
        folded={entry.folded.map(({ item }) => item)}
        item={item}
        key={id}
        leaving={folding?.ids.has(id) ?? false}
        onToggleStack={() => toggleStack(entry)}
        placement={entry.placement}
        revealOrder={revealed.has(id) ? revealOrder++ : undefined}
        section={section}
        stackOpen={!foldedStacks.has(id) && folding?.parent !== id}
        stackSize={entry.descendants.length}
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
  placement,
  folded,
  leaving,
  onToggleStack,
  revealOrder,
  stackOpen,
  stackSize,
}: Pick<RequestWorkspaceListProps, 'onAction' | 'params' | 'pendingId' | 'rail' | 'selectedId'> & {
  item: RequestQueueItemResponse
  section: RequestQueueSection
  placement: RequestStackPlacement
  folded: RequestQueueItemResponse[]
  leaving: boolean
  onToggleStack: () => void
  revealOrder: number | undefined
  stackOpen: boolean
  stackSize: number
}) {
  const hydrated = useHydrated()
  const { request, attention, author } = item
  const nowUnix = useUnixClock()
  const selected = selectedId === request.id
  const pending = pendingId === request.id
  const hot = attention.group === 'needs_you'
  const unread = requestHasNewActivity(item)
  const mergeStep = rail ? REQUEST_MERGE_STEP_LABELS[item.next_step] : undefined
  const stackParent = requestStackParentLabel(item)
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
  const foldedReady = folded.filter(({ next_step }) => next_step === 'merge').length
  return (
    <article
      className={cn('request-workspace-row', selected && 'request-workspace-row--selected')}
      data-heat={hot ? requestAttentionHeat(item.attention_at_unix, nowUnix) : 0}
      data-holds-selection={folded.some(({ request }) => request.id === selectedId) || undefined}
      data-leaving={leaving || undefined}
      data-rail={rail ? '' : undefined}
      data-request-id={request.id}
      data-revealed={revealOrder !== undefined || undefined}
      data-stack-folded={(stackSize > 0 && !stackOpen) || undefined}
      style={
        {
          '--row-actions': `${actionCount ? actionCount * 35 + 6 : 0}px`,
          '--stack-depth': placement.depth,
          '--reveal-order': revealOrder,
        } as CSSProperties
      }
    >
      <RequestStackLines placement={{ ...placement, children: placement.children && stackOpen }} />
      <Link
        aria-current={selected ? 'page' : undefined}
        className="request-workspace-row-link"
        params={{ ...params, requestId: request.id }}
        preload="intent"
        search={(current) => ({ view: current.view })}
        to="/$owner/$repo/requests/$requestId"
      >
        <span className="request-workspace-row-avatar">
          {stackSize > 0 && (
            <>
              <span aria-hidden="true" className="request-workspace-row-ghost" />
              <span aria-hidden="true" className="request-workspace-row-ghost" />
            </>
          )}
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
                attention.reason === 'authored' && 'request-workspace-row-hover-detail',
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
                {stackParent && <span className="sr-only">, {stackParent}</span>}
                {folded.length > 0 && (
                  <span className="request-workspace-row-hover-detail text-muted-foreground">
                    {' '}
                    · {folded.length} stacked
                    {foldedReady > 0 && <span className="text-success">, {foldedReady} ready</span>}
                  </span>
                )}
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
      {stackSize > 0 && (
        <button
          aria-expanded={stackOpen}
          aria-label={`${stackOpen ? 'Collapse' : 'Expand'} ${stackSize} stacked requests`}
          className="request-workspace-stack-toggle"
          onClick={onToggleStack}
          title={stackOpen ? 'Collapse stack' : 'Expand stack'}
          type="button"
        >
          <ChevronDown aria-hidden="true" />
        </button>
      )}
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
