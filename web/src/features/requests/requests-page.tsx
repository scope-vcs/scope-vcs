import type { RepoParams } from '@/api/types'
import type { RequestQueueSection } from '@/api/types.generated'
import { useAuth } from '@clerk/tanstack-react-start'
import { useNavigate, useParams } from '@tanstack/react-router'
import { useCallback, useState, type ReactNode } from 'react'
import { toast } from 'sonner'
import { loadRequestQueuePage, updateRequestAttention } from '@/routes/-request-workspace-actions'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { REQUEST_QUEUE_SECTION_ORDER } from './request-list-model'
import { loadMoreRequestQueue, requestQueueResource, searchRequestQueue, type LoadRequestQueuePage } from './request-queue-cache'
import { RequestWorkspaceShell, RequestWorkspaceSidebar, type RequestWorkspaceItem, type RequestWorkspaceSectionState } from './request-workspace-sidebar'
import { requestSnoozeUntil, requestWorkspaceItem } from './request-workspace-model'
import { RequestWorkspaceProvider } from './request-workspace-context'
import { useRequestQueue } from './use-request-queue'

export function RequestsPage({ children, params }: { children: ReactNode; params: RepoParams }) {
  const { userId, isLoaded } = useAuth()
  const { repo } = useRepoLayout()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  return <RequestWorkspaceContent key={scope ?? 'pending'} identity={scope} params={params} version={String(repo.change_version)}>{children}</RequestWorkspaceContent>
}

function RequestWorkspaceContent({ children, identity, params, version }: { children: ReactNode; identity: string | null; params: RepoParams; version: string }) {
  const navigate = useNavigate()
  const selected = useParams({ strict: false, select: (value) => value.requestId })
  const [collapsed, setCollapsed] = useState(false)
  const [draft, setDraft] = useState<string | null>(null)
  const [pendingId, setPendingId] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const load = useCallback<LoadRequestQueuePage>((section, cursor, search, signal) =>
    loadRequestQueuePage({ data: { owner: params.owner, repo: params.repo, section, cursor, search }, signal }), [params.owner, params.repo])
  const queue = useRequestQueue(identity, version, load)
  const query = queue.value?.query ?? ''
  const pages = queue.value?.pages

  const section = (key: RequestQueueSection): RequestWorkspaceSectionState => ({
    items: pages?.[key].requests.map((item) => requestWorkspaceItem(item, key, pendingId)) ?? [],
    hasMore: Boolean(pages?.[key].next_cursor),
    loading: queue.refreshing,
    error: queue.error ? 'Could not load requests. Try again.' : null,
    onRetry: () => {
      if (identity && draft !== null && draft.trim() !== query) void searchRequestQueue(identity, draft, load)
      else queue.retry()
    },
    onLoadMore: () => { if (identity) void loadMoreRequestQueue(identity, key, load) },
  })
  const active = section('active')
  if (query && pages) {
    active.items = REQUEST_QUEUE_SECTION_ORDER.flatMap((key) => section(key).items)
    active.hasMore = REQUEST_QUEUE_SECTION_ORDER.some((key) => Boolean(pages[key].next_cursor))
    active.onLoadMore = () => {
      const key = REQUEST_QUEUE_SECTION_ORDER.find((key) => pages[key].next_cursor)
      if (identity && key) void loadMoreRequestQueue(identity, key, load)
    }
  }

  const rows = pages ? REQUEST_QUEUE_SECTION_ORDER.flatMap((key) => pages[key].requests) : []
  const selectedIndex = rows.findIndex((item) => item.request.id === selected)
  const selectedRow = rows[selectedIndex] ?? null

  async function act(item: RequestWorkspaceItem, action: 'claim' | 'restore' | 'settle' | 'snooze', until?: number) {
    if (!identity || pendingId || !pages) return
    const row = REQUEST_QUEUE_SECTION_ORDER.flatMap((key) => pages[key].requests).find((entry) => entry.request.id === item.id)
    if (!row) return
    setPendingId(item.id)
    setActionError(null)
    try {
      const common = { ...params, request_id: item.id, expected_activity_version: row.attention.activity_version }
      const result = await updateRequestAttention({ data: action === 'snooze' ? { ...common, action, until_unix: until! } : { ...common, action } })
      requestQueueResource.invalidate(identity)
      const message = { claim: 'Request claimed', restore: 'Request restored', settle: 'Request settled', snooze: 'Request snoozed' }[action]
      toast.success(message, action === 'settle' || action === 'snooze' ? {
        action: { label: 'Undo', onClick: () => {
          void updateRequestAttention({ data: { ...params, request_id: item.id, action: 'restore', expected_activity_version: result.attention.activity_version } }).then(() => {
            requestQueueResource.invalidate(identity)
            void navigate({ to: '/$owner/$repo/requests/$requestId', params: { ...params, requestId: item.id } })
          }).catch((error: unknown) => toast.error(error instanceof Error ? error.message : 'Could not restore request.'))
        } },
      } : undefined)
      if (action === 'claim') {
        await navigate({ to: '/$owner/$repo/requests/$requestId', params: { ...params, requestId: item.id } })
      } else if ((action === 'settle' || action === 'snooze') && selected === item.id) {
        const remaining = pages.active.requests.filter((entry) => entry.request.id !== item.id)
        const next = remaining[Math.min(Math.max(0, pages.active.requests.findIndex((entry) => entry.request.id === item.id)), remaining.length - 1)]
        if (next) await navigate({ to: '/$owner/$repo/requests/$requestId', params: { ...params, requestId: next.request.id } })
        else await navigate({ to: '/$owner/$repo/requests', params })
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Could not update request attention.'
      setActionError(message)
      toast.error(message)
      requestQueueResource.invalidate(identity)
    } finally {
      setPendingId(null)
    }
  }

  return (
    <RequestWorkspaceShell collapsed={collapsed} detailOpenOnMobile={Boolean(selected)} sidebar={(
      <RequestWorkspaceSidebar
        active={active} unclaimed={section('unclaimed')} setAside={section('set_aside')}
        collapsed={collapsed} onCollapsedChange={setCollapsed} params={params} selectedRequestId={selected}
        searchValue={draft ?? queue.value?.requestedQuery ?? query} searchQuery={query} searchBusy={queue.refreshing}
        searchError={actionError} onSearchValueChange={setDraft}
        onSearchSubmit={(value) => { if (identity) void searchRequestQueue(identity, value, load) }}
        onClaim={(item) => void act(item, 'claim')} onRestore={(item) => void act(item, 'restore')}
        onSettle={(item) => void act(item, 'settle')}
        onSnooze={(item, option) => void act(item, 'snooze', requestSnoozeUntil(option.value))}
        snoozeOptions={[{ label: 'In an hour', value: 'hour' }, { label: 'Tomorrow', detail: '9:00 AM', value: 'tomorrow' }, { label: 'Next week', detail: 'Monday, 9:00 AM', value: 'next_week' }]}
      />
    )}>
      <RequestWorkspaceProvider value={{
        selected: selectedRow,
        previousId: rows[selectedIndex - 1]?.request.id ?? null,
        nextId: rows[selectedIndex + 1]?.request.id ?? null,
        claim: () => { if (selectedRow) void act(requestWorkspaceItem(selectedRow, 'unclaimed', pendingId), 'claim') },
      }}>{children}</RequestWorkspaceProvider>
    </RequestWorkspaceShell>
  )
}
