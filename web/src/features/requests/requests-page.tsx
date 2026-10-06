import type { RepoParams } from '@/api/types'
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
  searchRequestQueue,
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
  const [draft, setDraft] = useState<string | null>(null)
  useEffect(() => {
    document.documentElement.toggleAttribute('data-focus', focus)
    return () => document.documentElement.removeAttribute('data-focus')
  }, [focus])
  const toggleFocus = useCallback(() => setFocus((value) => !value), [])
  const load = useCallback<LoadRequestQueuePage>(
    (section, cursor, search, signal) =>
      loadRequestQueuePage({
        data: { owner: params.owner, repo: params.repo, section, cursor, search, view },
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
  const query = draft ?? queue.value?.requestedQuery ?? ''
  const pages = useMemo(
    () => loadedPages && applyAttentionMoves(loadedPages, moves),
    [loadedPages, moves],
  )
  const selected =
    REQUEST_QUEUE_SECTION_ORDER.flatMap((section) => pages?.[section].requests ?? []).find(
      (item) => item.request.id === selectedId,
    ) ?? null

  function search(value: string) {
    setDraft(value)
    if (identity && value.trim() !== queue.value?.requestedQuery)
      void searchRequestQueue(identity, value, load)
  }

  function changeCollapsed(value: boolean) {
    setCollapsed(value)
    saveRequestWorkspaceCollapsed(value)
    if (!value) setFocus(false)
    else if (query) search('')
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
          skeleton={!queue.value || (queue.refreshing && queue.value.query !== queue.value.requestedQuery)}
          maintainer={maintainer}
          onAction={(item, command) => void act(item, command)}
          onCollapsedChange={changeCollapsed}
          onFocusToggle={toggleFocus}
          onLoadMore={(section) => {
            if (identity) void loadMoreRequestQueue(identity, section, load)
          }}
          onRetry={() => {
            if (identity && query.trim() !== queue.value?.query)
              void searchRequestQueue(identity, query, load)
            else queue.retry()
          }}
          onSearch={search}
          pages={pages}
          params={params}
          pendingId={pendingId}
          query={query}
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
    </RequestWorkspaceShell>
  )
}
