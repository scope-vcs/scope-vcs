import type { RepoParams } from '@/api/types'
import { useAuth } from '@clerk/tanstack-react-start'
import { useParams } from '@tanstack/react-router'
import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react'
import { loadRequestQueuePage } from '@/routes/-request-workspace-actions'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { REQUEST_QUEUE_SECTION_ORDER } from './request-list-model'
import {
  loadMoreRequestQueue,
  searchRequestQueue,
  type LoadRequestQueuePage,
} from './request-queue-cache'
import { RequestWorkspaceSidebar } from './request-workspace-sidebar'
import { RequestWorkspaceShell } from './request-workspace-shell'
import { RequestWorkspaceProvider } from './request-workspace-context'
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
  identity,
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
  const [collapsed, setCollapsed] = useState(false)
  const [focus, setFocus] = useState(false)
  const [draft, setDraft] = useState<string | null>(null)
  // Focus mode hides the app chrome, which lives above this page, so the
  // page announces it on the document and leaves when it unmounts.
  useEffect(() => {
    document.documentElement.toggleAttribute('data-focus', focus)
    return () => document.documentElement.removeAttribute('data-focus')
  }, [focus])
  const toggleFocus = useCallback(() => setFocus((value) => !value), [])
  const load = useCallback<LoadRequestQueuePage>(
    (section, cursor, search, signal) =>
      loadRequestQueuePage({
        data: { owner: params.owner, repo: params.repo, section, cursor, search },
        signal,
      }),
    [params.owner, params.repo],
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
    if (!value) setFocus(false)
    // The rail draws the queue, so collapsing drops any search.
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
          // Placeholders stand in only while there is nothing true to show:
          // the first load, or a search whose results have not arrived.
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
