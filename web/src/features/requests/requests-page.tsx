import type { RepoParams } from '@/api/types'
import type { RequestQueueItemResponse } from '@/api/types.generated'
import { useAuth } from '@clerk/tanstack-react-start'
import { useParams } from '@tanstack/react-router'
import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react'
import { loadRequestQueuePage } from '@/routes/-request-workspace-actions'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { RepoViewingAsPicker, useViewingAs } from '../repo-detail/use-viewing-as'
import { REQUEST_QUEUE_SECTION_ORDER } from './request-list-model'
import {
  loadMoreRequestQueue,
  requestQueueIdentity,
  type LoadRequestQueuePage,
} from './request-queue-cache'
import { RequestWorkspaceSidebar } from './request-workspace-sidebar'
import { RequestWorkspaceShell } from './request-workspace-shell'
import { RequestWorkspaceProvider } from './request-workspace-context'
import {
  readRequestWorkspaceCollapsed,
  saveRequestWorkspaceCollapsed,
} from './request-workspace-collapse'
import { applyAttentionMoves } from './request-attention-moves'
import { RequestCustomSnoozeDialog } from './request-custom-snooze-dialog'
import { useRequestAttentionActions } from './use-request-attention-actions'
import { useRequestQueue } from './use-request-queue'

export function RequestsPage({ children, params }: { children: ReactNode; params: RepoParams }) {
  const { userId, isLoaded } = useAuth()
  const { repo } = useRepoLayout()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  return (
    <RequestWorkspaceContent
      key={scope ?? 'pending'}
      identity={scope}
      maintainer={repo.access.actor !== 'Public'}
      params={params}
      version={String(repo.change_version)}
    >
      {children}
    </RequestWorkspaceContent>
  )
}

function RequestWorkspaceContent({
  children,
  identity: scope,
  maintainer,
  params,
  version,
}: {
  children: ReactNode
  identity: string | null
  maintainer: boolean
  params: RepoParams
  version: string
}) {
  const selectedId = useParams({ strict: false, select: (value) => value.requestId })
  const { options, view } = useViewingAs()
  const identity = scope && requestQueueIdentity(scope, view)
  const [collapsed, setCollapsed] = useState(readRequestWorkspaceCollapsed)
  const [focus, setFocus] = useState(false)
  const [customSnoozeItem, setCustomSnoozeItem] = useState<RequestQueueItemResponse | null>(null)
  useEffect(() => {
    document.documentElement.toggleAttribute('data-focus', focus)
    return () => document.documentElement.removeAttribute('data-focus')
  }, [focus])
  const toggleFocus = useCallback(() => setFocus((value) => !value), [])
  const load = useCallback<LoadRequestQueuePage>(
    (section, cursor, signal) =>
      loadRequestQueuePage({
        data: { owner: params.owner, repo: params.repo, section, cursor, search: null, view },
        signal,
      }),
    [params.owner, params.repo, view],
  )
  const queue = useRequestQueue(identity, version, load)
  const loadedPages = queue.value?.pages
  const { act, error, moves, pendingId } = useRequestAttentionActions(
    identity,
    params,
    loadedPages,
    selectedId,
  )
  const pages = useMemo(
    () => loadedPages && applyAttentionMoves(loadedPages, moves),
    [loadedPages, moves],
  )
  const selected =
    REQUEST_QUEUE_SECTION_ORDER.flatMap((section) => pages?.[section].requests ?? []).find(
      (item) => item.request.id === selectedId,
    ) ?? null

  function changeCollapsed(value: boolean) {
    setCollapsed(value)
    saveRequestWorkspaceCollapsed(value)
    if (!value) setFocus(false)
  }

  return (
    <RequestWorkspaceShell
      collapsed={collapsed || focus}
      detailOpenOnMobile={Boolean(selectedId)}
      focus={focus}
      onCollapsedChange={changeCollapsed}
      sidebar={
        <RequestWorkspaceSidebar
          actionError={error}
          collapsed={collapsed || focus}
          focus={focus}
          error={queue.error ? 'Could not load requests. Try again.' : null}
          loading={queue.refreshing}
          skeleton={!queue.value}
          maintainer={maintainer}
          onAction={(item, command) => void act(item, command)}
          onCustomSnooze={setCustomSnoozeItem}
          onCollapsedChange={changeCollapsed}
          onFocusToggle={toggleFocus}
          onLoadMore={(section) => {
            if (identity) void loadMoreRequestQueue(identity, section, load)
          }}
          onRetry={queue.retry}
          pages={pages}
          params={params}
          pendingId={pendingId}
          selectedId={selectedId}
          viewingAs={options.length > 1 ? <RepoViewingAsPicker compact /> : null}
        />
      }
    >
      <RequestWorkspaceProvider
        value={{
          selected,
          claim: () => {
            if (selected) void act(selected, { action: 'claim' })
          },
          release: () => {
            if (selected) void act(selected, { action: 'release' })
          },
        }}
      >
        {children}
      </RequestWorkspaceProvider>
      {customSnoozeItem && (
        <RequestCustomSnoozeDialog
          onClose={() => setCustomSnoozeItem(null)}
          onSnooze={(until_unix) => void act(customSnoozeItem, { action: 'snooze', until_unix })}
        />
      )}
    </RequestWorkspaceShell>
  )
}
