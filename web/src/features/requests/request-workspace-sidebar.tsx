import type { RepoParams } from '@/api/types'
import { Button } from '@/components/ui/button'
import { BlockSkeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import {
  Check,
  ChevronRight,
  Clock3,
  Inbox,
  LoaderCircle,
  PanelLeftClose,
  PanelLeftOpen,
  RefreshCw,
  Search,
  UserRound,
  X,
} from 'lucide-react'
import {
  type FormEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  useEffect,
  useId,
  useRef,
  useState,
} from 'react'
import './request-workspace-sidebar.css'

export type RequestWorkspaceSection = 'active' | 'unclaimed' | 'set_aside'

export type RequestWorkspaceItem = {
  actionPending?: boolean
  authorInitials?: string
  authorName: string
  badgeLabel?: string
  canClaim?: boolean
  canRestore?: boolean
  canSettle?: boolean
  canSnooze?: boolean
  id: string
  reason: string
  section: RequestWorkspaceSection
  timeLabel: string
  title: string
  unread?: boolean
}

export type RequestWorkspaceSectionState = {
  count?: number
  emptyLabel?: string
  error?: string | null
  hasMore?: boolean
  items: RequestWorkspaceItem[]
  loading?: boolean
  onLoadMore?: () => void
  onRetry?: () => void
}

export type RequestWorkspaceSnoozeOption = {
  detail?: string
  label: string
  value: string
}

export type RequestWorkspaceSidebarProps = {
  active: RequestWorkspaceSectionState
  collapsed: boolean
  onClaim?: (item: RequestWorkspaceItem) => void
  onCollapsedChange: (collapsed: boolean) => void
  onRestore?: (item: RequestWorkspaceItem) => void
  onSearchSubmit?: (value: string) => void
  onSearchValueChange: (value: string) => void
  onSettle?: (item: RequestWorkspaceItem) => void
  onSnooze?: (
    item: RequestWorkspaceItem,
    option: RequestWorkspaceSnoozeOption,
  ) => void
  params: RepoParams
  searchBusy?: boolean
  searchError?: string | null
  searchQuery?: string
  searchValue: string
  selectedRequestId?: string | null
  setAside: RequestWorkspaceSectionState
  snoozeOptions?: RequestWorkspaceSnoozeOption[]
  unclaimed: RequestWorkspaceSectionState
}

export type RequestWorkspaceShellProps = {
  children: ReactNode
  collapsed: boolean
  detailOpenOnMobile: boolean
  sidebar: ReactNode
}

const DEFAULT_SNOOZE_OPTIONS: RequestWorkspaceSnoozeOption[] = [
  { detail: '9:00 AM', label: 'Tomorrow', value: 'tomorrow' },
  { detail: 'Monday', label: 'Next week', value: 'next_week' },
]

function moveSnoozeMenuFocus(event: ReactKeyboardEvent<HTMLDivElement>) {
  if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return
  const buttons = Array.from(
    event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
  )
  if (!buttons.length) return
  event.preventDefault()
  const currentIndex = buttons.indexOf(document.activeElement as HTMLButtonElement)
  const nextIndex = event.key === 'Home'
    ? 0
    : event.key === 'End'
      ? buttons.length - 1
      : event.key === 'ArrowDown'
        ? (currentIndex + 1) % buttons.length
        : (currentIndex - 1 + buttons.length) % buttons.length
  buttons[nextIndex]?.focus()
}

export function RequestWorkspaceShell({
  children,
  collapsed,
  detailOpenOnMobile,
  sidebar,
}: RequestWorkspaceShellProps) {
  return (
    <div
      className="request-workspace-shell"
      data-collapsed={collapsed || undefined}
      data-detail-open={detailOpenOnMobile || undefined}
    >
      {sidebar}
      <section className="request-workspace-detail">{children}</section>
    </div>
  )
}

export function RequestWorkspaceSidebar({
  active,
  collapsed,
  onClaim,
  onCollapsedChange,
  onRestore,
  onSearchSubmit,
  onSearchValueChange,
  onSettle,
  onSnooze,
  params,
  searchBusy = false,
  searchError,
  searchQuery = '',
  searchValue,
  selectedRequestId,
  setAside,
  snoozeOptions = DEFAULT_SNOOZE_OPTIONS,
  unclaimed,
}: RequestWorkspaceSidebarProps) {
  const [unclaimedOpen, setUnclaimedOpen] = useState(false)
  const [setAsideOpen, setSetAsideOpen] = useState(false)
  const [snoozeMenu, setSnoozeMenu] = useState<SnoozeMenuState | null>(null)
  const snoozeMenuRef = useRef<HTMLDivElement>(null)
  const snoozeTriggerRef = useRef<HTMLButtonElement | null>(null)
  const searchId = useId()
  const searching = searchQuery.trim().length > 0

  useEffect(() => {
    if (!snoozeMenu) return

    snoozeMenuRef.current?.querySelector<HTMLButtonElement>('button')?.focus()

    function closeSnoozeMenu() {
      setSnoozeMenu(null)
    }

    function closeFromPointer(event: PointerEvent) {
      const target = event.target
      if (!(target instanceof Node)) return
      if (snoozeMenuRef.current?.contains(target)) return
      if (snoozeTriggerRef.current?.contains(target)) return
      closeSnoozeMenu()
    }

    function closeFromKeyboard(event: KeyboardEvent) {
      if (event.key !== 'Escape') return
      closeSnoozeMenu()
      snoozeTriggerRef.current?.focus()
    }

    function closeFromViewportChange() {
      closeSnoozeMenu()
    }

    document.addEventListener('pointerdown', closeFromPointer)
    document.addEventListener('keydown', closeFromKeyboard)
    window.addEventListener('resize', closeFromViewportChange)
    window.addEventListener('scroll', closeFromViewportChange, true)
    return () => {
      document.removeEventListener('pointerdown', closeFromPointer)
      document.removeEventListener('keydown', closeFromKeyboard)
      window.removeEventListener('resize', closeFromViewportChange)
      window.removeEventListener('scroll', closeFromViewportChange, true)
    }
  }, [snoozeMenu])

  function submitSearch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    onSearchSubmit?.(searchValue)
  }

  function clearSearch() {
    onSearchValueChange('')
    onSearchSubmit?.('')
  }

  function openSnoozeMenu(
    item: RequestWorkspaceItem,
    trigger: HTMLButtonElement,
  ) {
    if (snoozeMenu?.item.id === item.id) {
      setSnoozeMenu(null)
      return
    }

    const bounds = trigger.getBoundingClientRect()
    const menuWidth = 248
    const estimatedHeight = 58 + (snoozeOptions.length * 42)
    const roomBelow = window.innerHeight - bounds.bottom
    const top = roomBelow >= estimatedHeight + 12
      ? bounds.bottom + 7
      : Math.max(8, bounds.top - estimatedHeight - 7)
    const left = Math.max(8, Math.min(bounds.right - menuWidth, window.innerWidth - menuWidth - 8))
    snoozeTriggerRef.current = trigger
    setSnoozeMenu({ item, left, top })
  }

  return (
    <aside
      aria-label="Requests workspace"
      className={cn(
        'request-workspace-sidebar',
        collapsed && 'request-workspace-sidebar--collapsed',
      )}
    >
      <div className="request-workspace-collapsed-view">
        <Button
          aria-label="Expand requests sidebar"
          onClick={() => onCollapsedChange(false)}
          size="icon-sm"
          title="Expand requests sidebar"
          type="button"
          variant="ghost"
        >
          <PanelLeftOpen />
        </Button>
        <Inbox aria-hidden="true" className="mt-1 size-4 text-muted-foreground" />
        <span className="font-mono text-[10px] text-success-strong">
          {sectionCountLabel(active)}
        </span>
      </div>

      <div className="request-workspace-expanded-view">
        <div className="request-workspace-sidebar-tools">
          <form className="request-workspace-search" onSubmit={submitSearch} role="search">
            <label className="sr-only" htmlFor={searchId}>Search requests</label>
            <Search aria-hidden="true" className="request-workspace-search-icon" />
            <input
              aria-describedby={searchError ? `${searchId}-error` : undefined}
              autoComplete="off"
              disabled={searchBusy}
              id={searchId}
              onChange={(event) => onSearchValueChange(event.target.value)}
              placeholder="Search requests"
              type="search"
              value={searchValue}
            />
            {searchValue ? (
              <button
                aria-label="Clear request search"
                className="request-workspace-search-clear"
                disabled={searchBusy}
                onClick={clearSearch}
                type="button"
              >
                <X aria-hidden="true" />
              </button>
            ) : null}
            {searchBusy ? (
              <LoaderCircle aria-label="Searching requests" className="request-workspace-search-spinner" />
            ) : null}
          </form>
          <Button
            aria-label="Collapse requests sidebar"
            onClick={() => {
              setSnoozeMenu(null)
              onCollapsedChange(true)
            }}
            size="icon-sm"
            title="Collapse requests sidebar"
            type="button"
            variant="ghost"
          >
            <PanelLeftClose />
          </Button>
        </div>

        {searchError ? (
          <p className="request-workspace-search-error" id={`${searchId}-error`} role="alert">
            {searchError}
          </p>
        ) : null}

        <div aria-busy={searchBusy} className="request-workspace-scroll">
          <RequestWorkspaceList
            emptyLabel={searching ? 'No matching requests.' : 'You’re caught up.'}
            onClaim={onClaim}
            onRestore={onRestore}
            onSettle={onSettle}
            onSnoozeOpen={onSnooze ? openSnoozeMenu : undefined}
            params={params}
            section={active}
            selectedRequestId={selectedRequestId}
            snoozeOpenId={snoozeMenu?.item.id}
          />

          {!searching ? (
            <div className="request-workspace-shelves">
              <RequestWorkspaceDisclosure
                label="Unclaimed"
                onOpenChange={setUnclaimedOpen}
                open={unclaimedOpen}
                section={unclaimed}
                selectedRequestId={selectedRequestId}
              >
                <RequestWorkspaceList
                  emptyLabel="Every request has a maintainer."
                  onClaim={onClaim}
                  onRestore={onRestore}
                  onSettle={onSettle}
                  onSnoozeOpen={onSnooze ? openSnoozeMenu : undefined}
                  params={params}
                  section={unclaimed}
                  selectedRequestId={selectedRequestId}
                  snoozeOpenId={snoozeMenu?.item.id}
                />
              </RequestWorkspaceDisclosure>
              <RequestWorkspaceDisclosure
                label="Set aside"
                onOpenChange={setSetAsideOpen}
                open={setAsideOpen}
                section={setAside}
                selectedRequestId={selectedRequestId}
              >
                <RequestWorkspaceList
                  emptyLabel="Nothing set aside."
                  onClaim={onClaim}
                  onRestore={onRestore}
                  onSettle={onSettle}
                  onSnoozeOpen={onSnooze ? openSnoozeMenu : undefined}
                  params={params}
                  section={setAside}
                  selectedRequestId={selectedRequestId}
                  snoozeOpenId={snoozeMenu?.item.id}
                />
              </RequestWorkspaceDisclosure>
            </div>
          ) : null}
        </div>

        {snoozeMenu && onSnooze ? (
          <div
            aria-label={`Snooze request ${snoozeMenu.item.id}`}
            className="request-workspace-snooze-menu"
            onKeyDown={moveSnoozeMenuFocus}
            ref={snoozeMenuRef}
            role="menu"
            style={{ left: snoozeMenu.left, top: snoozeMenu.top }}
            tabIndex={-1}
          >
            <p>Remind me about #{snoozeMenu.item.id}</p>
            {snoozeOptions.map((option) => (
              <button
                key={option.value}
                onClick={() => {
                  setSnoozeMenu(null)
                  onSnooze(snoozeMenu.item, option)
                }}
                role="menuitem"
                type="button"
              >
                <span>{option.label}</span>
                {option.detail ? <small>{option.detail}</small> : null}
              </button>
            ))}
          </div>
        ) : null}
      </div>
    </aside>
  )
}

type SnoozeMenuState = {
  item: RequestWorkspaceItem
  left: number
  top: number
}

type RequestWorkspaceListProps = Pick<
  RequestWorkspaceSidebarProps,
  'onClaim' | 'onRestore' | 'onSettle' | 'params' | 'selectedRequestId'
> & {
  emptyLabel: string
  onSnoozeOpen?: (item: RequestWorkspaceItem, trigger: HTMLButtonElement) => void
  section: RequestWorkspaceSectionState
  snoozeOpenId?: string
}

function RequestWorkspaceList({
  emptyLabel,
  onClaim,
  onRestore,
  onSettle,
  onSnoozeOpen,
  params,
  section,
  selectedRequestId,
  snoozeOpenId,
}: RequestWorkspaceListProps) {
  if (!section.items.length && section.loading) {
    return (
      <div aria-label="Loading requests" className="divide-y divide-border">
        {[0, 1, 2].map((index) => (
          <div className="space-y-3 px-5 py-4" key={index}>
            <BlockSkeleton className="h-4 w-4/5" />
            <BlockSkeleton className="h-3 w-3/5" />
          </div>
        ))}
      </div>
    )
  }

  return (
    <div className="request-workspace-list">
      {section.items.map((item) => (
        <RequestWorkspaceRow
          item={item}
          key={item.id}
          onClaim={onClaim}
          onRestore={onRestore}
          onSettle={onSettle}
          onSnoozeOpen={onSnoozeOpen}
          params={params}
          selected={selectedRequestId === item.id}
          snoozeOpen={snoozeOpenId === item.id}
        />
      ))}
      {!section.items.length && !section.error ? (
        <p className="request-workspace-empty">{section.emptyLabel ?? emptyLabel}</p>
      ) : null}
      {section.error ? (
        <div className="request-workspace-list-error" role="alert">
          <p>{section.error}</p>
          {section.onRetry ? (
            <Button onClick={section.onRetry} size="sm" type="button" variant="ghost">
              Try again
            </Button>
          ) : null}
        </div>
      ) : null}
      {section.hasMore && section.onLoadMore ? (
        <Button
          className="request-workspace-load-more"
          disabled={section.loading}
          onClick={section.onLoadMore}
          size="sm"
          type="button"
          variant="ghost"
        >
          {section.loading ? (
            <><LoaderCircle className="animate-spin" />Loading…</>
          ) : 'Load more'}
        </Button>
      ) : null}
    </div>
  )
}

function RequestWorkspaceDisclosure({
  children,
  label,
  onOpenChange,
  open,
  section,
  selectedRequestId,
}: {
  children: ReactNode
  label: string
  onOpenChange: (open: boolean) => void
  open: boolean
  section: RequestWorkspaceSectionState
  selectedRequestId?: string | null
}) {
  const contentId = useId()
  const selectedInside = section.items.some((item) => item.id === selectedRequestId)
  return (
    <section className="request-workspace-shelf">
      <button
        aria-controls={contentId}
        aria-expanded={open}
        className="request-workspace-shelf-toggle"
        onClick={() => onOpenChange(!open)}
        type="button"
      >
        <ChevronRight aria-hidden="true" />
        <span>{label}</span>
        {selectedInside && !open ? (
          <span aria-label="Selected request is in this section" className="request-workspace-selected-dot" />
        ) : null}
        <span className="request-workspace-count">{sectionCountLabel(section)}</span>
      </button>
      <div hidden={!open} id={contentId}>{children}</div>
    </section>
  )
}

type RequestWorkspaceRowProps = Pick<
  RequestWorkspaceSidebarProps,
  'onClaim' | 'onRestore' | 'onSettle' | 'params'
> & {
  item: RequestWorkspaceItem
  onSnoozeOpen?: (item: RequestWorkspaceItem, trigger: HTMLButtonElement) => void
  selected: boolean
  snoozeOpen: boolean
}

function RequestWorkspaceRow({
  item,
  onClaim,
  onRestore,
  onSettle,
  onSnoozeOpen,
  params,
  selected,
  snoozeOpen,
}: RequestWorkspaceRowProps) {
  const canClaim = item.canClaim && onClaim
  const canRestore = item.canRestore && onRestore
  const canSettle = item.canSettle && onSettle
  const canSnooze = item.canSnooze && onSnoozeOpen
  const hasActions = canClaim || canRestore || canSettle || canSnooze

  return (
    <article
      className={cn(
        'request-workspace-row',
        selected && 'request-workspace-row--selected',
        item.section === 'set_aside' && 'request-workspace-row--quiet',
      )}
    >
      <Link
        aria-current={selected ? 'page' : undefined}
        className={cn('request-workspace-row-link', hasActions && 'request-workspace-row-link--actions')}
        params={{ ...params, requestId: item.id }}
        preload="intent"
        search={{}}
        to="/$owner/$repo/requests/$requestId"
      >
        <span className="request-workspace-row-title">{item.title}</span>
        <span className="request-workspace-row-meta">
          <span className="request-workspace-reason">
            <span
              aria-hidden="true"
              className={cn(
                'request-workspace-reason-dot',
                item.unread && 'request-workspace-reason-dot--unread',
              )}
            />
            <span>{item.reason}</span>
          </span>
          <span className="request-workspace-row-time">{item.timeLabel}</span>
        </span>
        <span className="request-workspace-row-footer">
          {item.authorInitials ? (
            <span aria-hidden="true" className="request-workspace-avatar">{item.authorInitials}</span>
          ) : null}
          <span>{item.authorName}</span>
          <span aria-hidden="true">·</span>
          <span className="font-mono">#{item.id}</span>
          {item.badgeLabel ? <span className="request-workspace-row-badge">{item.badgeLabel}</span> : null}
        </span>
      </Link>
      {hasActions ? (
        <fieldset className="request-workspace-row-actions">
          <legend className="sr-only">Request {item.id} actions</legend>
          {canSnooze ? (
            <RowAction
              expanded={snoozeOpen}
              hasPopup="menu"
              disabled={item.actionPending}
              icon={<Clock3 />}
              label={`Snooze request ${item.id}`}
              onClick={(event) => onSnoozeOpen(item, event.currentTarget)}
            />
          ) : null}
          {canSettle ? (
            <RowAction
              disabled={item.actionPending}
              icon={<Check />}
              label={`Settle request ${item.id}`}
              onClick={() => onSettle(item)}
            />
          ) : null}
          {canClaim ? (
            <RowAction
              disabled={item.actionPending}
              icon={<UserRound />}
              label={`Claim request ${item.id}`}
              onClick={() => onClaim(item)}
            />
          ) : null}
          {canRestore ? (
            <RowAction
              disabled={item.actionPending}
              icon={<RefreshCw />}
              label={`Restore request ${item.id}`}
              onClick={() => onRestore(item)}
            />
          ) : null}
        </fieldset>
      ) : null}
    </article>
  )
}

function RowAction({
  disabled,
  expanded,
  hasPopup,
  icon,
  label,
  onClick,
}: {
  disabled?: boolean
  expanded?: boolean
  hasPopup?: 'menu'
  icon: ReactNode
  label: string
  onClick: (event: React.MouseEvent<HTMLButtonElement>) => void
}) {
  return (
    <button
      aria-label={label}
      aria-expanded={hasPopup ? expanded : undefined}
      aria-haspopup={hasPopup}
      className="request-workspace-row-action"
      disabled={disabled}
      onClick={onClick}
      title={label}
      type="button"
    >
      {disabled ? <LoaderCircle className="animate-spin" /> : icon}
    </button>
  )
}

function sectionCountLabel(section: RequestWorkspaceSectionState) {
  return `${section.count ?? section.items.length}${section.hasMore ? '+' : ''}`
}
