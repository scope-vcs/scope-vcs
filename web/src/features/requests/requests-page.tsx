import type { RepoParams } from '@/api/types'
import { useAuth } from '@clerk/tanstack-react-start'
import { useParams } from '@tanstack/react-router'
import { useCallback, useState, type ReactNode } from 'react'
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
  params,
  version,
}: {
  children: ReactNode
  identity: string | null
  params: RepoParams
  version: string
}) {
  const selectedId = useParams({ strict: false, select: (value) => value.requestId })
  const [collapsed, setCollapsed] = useState(false)
  const [draft, setDraft] = useState<string | null>(null)
  const load = useCallback<LoadRequestQueuePage>(
    (section, cursor, search, signal) =>
      loadRequestQueuePage({
        data: { owner: params.owner, repo: params.repo, section, cursor, search },
        signal,
      }),
    [params.owner, params.repo],
  )
  const queue = useRequestQueue(identity, version, load)
  const { act, error, pendingId } = useRequestAttentionActions(identity, params, selectedId)
  const query = draft ?? queue.value?.requestedQuery ?? ''
  const pages = queue.value?.pages
  const selected =
    REQUEST_QUEUE_SECTION_ORDER.flatMap((section) => pages?.[section].requests ?? []).find(
      (item) => item.request.id === selectedId,
    ) ?? null

  function search(value: string) {
    setDraft(value)
    if (identity && value.trim() !== queue.value?.requestedQuery)
      void searchRequestQueue(identity, value, load)
  }

  return (
    <RequestWorkspaceShell
      collapsed={collapsed}
      detailOpenOnMobile={Boolean(selectedId)}
      onCollapsedChange={setCollapsed}
      sidebar={
        <RequestWorkspaceSidebar
          actionError={error}
          collapsed={collapsed}
          error={queue.error ? 'Could not load requests. Try again.' : null}
          loading={queue.refreshing}
          onAction={(item, command) => void act(item, command)}
          onCollapsedChange={setCollapsed}
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
        }}
      >
        {children}
      </RequestWorkspaceProvider>
    </RequestWorkspaceShell>
  )
}
