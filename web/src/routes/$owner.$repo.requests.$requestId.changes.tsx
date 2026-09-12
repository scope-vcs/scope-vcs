import {
  parseLoadRequestRevisionsInput,
  parseLoadRequestRevisionDiffInput,
  parseLoadDiscussionsInput,
} from '@/api/request-inputs'
import {
  type LoadRequestRevisionCommitInput,
  loadRequestRevisionCommitFileDiffForRequest,
  loadRequestRevisionsForRequest,
} from '@/api/requests'
import { EmptyState } from '@/components/empty-state'
import { Button } from '@/components/ui/button'
import {
  type LoadDiscussionsInput,
  loadRequestDiscussionsForRequest,
} from '@/features/requests/request-discussion-api'
import { loadDiscussionReferencePage, selectedDiscussionReferenceQuery } from '@/features/requests/request-changes-discussion-references'
import { requestChangesSelectionIdentity, requestChangesResource } from '@/features/requests/request-changes-resource'
import { useRequestChangesResource } from '@/features/requests/use-request-changes-resource'
import {
  RequestChangesWorkbench,
  type RequestChangesDiscussionReferences,
  type RequestChangesSearch,
} from '@/features/requests/request-changes-workbench'
import { RequestChangesPending } from '@/features/requests/request-page-pending'
import {
  requestChangeSelection,
  requestRevisionPin,
} from '@/features/requests/request-changes-model'
import { requestParamsForRoute } from '@/features/requests/request-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { Link, createFileRoute, getRouteApi } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'
import { GitCommit } from 'lucide-react'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback, useEffect, useMemo } from 'react'

const requestRoute = getRouteApi('/$owner/$repo/requests/$requestId')

const loadChangesPage = createServerFn({ method: 'GET' })
  .validator(parseLoadRequestRevisionsInput)
  .handler(async ({ data }) => {
    const revisions = await loadRequestRevisionsForRequest(data).catch((error: unknown) => {
      console.error('Loading request revisions failed', error)
      return null
    })
    const query = revisions ? selectedDiscussionReferenceQuery(data, revisions) : null
    const page = query
      ? await loadDiscussionReferencePage(query.input, loadRequestDiscussionsForRequest)
        .catch((error: unknown) => {
          console.error('Loading request discussion references failed', error)
          return null
        })
      : null
    const discussionReferences: RequestChangesDiscussionReferences = {
      commitKey: query?.key ?? null,
      page,
    }
    return { discussionReferences, revisions }
  })

const loadRevisionDiff = createServerFn({ method: 'GET' })
  .validator(parseLoadRequestRevisionDiffInput)
  .handler(({ data }) => loadRequestRevisionCommitFileDiffForRequest(data, getRequest().signal))

const loadRevisions = createServerFn({ method: 'GET' })
  .validator(parseLoadRequestRevisionsInput)
  .handler(({ data }) => loadRequestRevisionsForRequest(data))

const loadDiscussions = createServerFn({ method: 'GET' })
  .validator(parseLoadDiscussionsInput)
  .handler(({ data }) => loadDiscussionReferencePage(data, loadRequestDiscussionsForRequest))

const loadDiffForView = (
  data: LoadRequestRevisionCommitInput & { path: string },
  signal?: AbortSignal,
) => loadRevisionDiff({ data, signal })
const loadDiscussionsForView = (data: LoadDiscussionsInput) =>
  loadDiscussions({ data })

export const Route = createFileRoute(
  '/$owner/$repo/requests/$requestId/changes',
)({
  loaderDeps: ({ search }) => requestChangesSelectionSearch(search),
  loader: async ({ deps: selectionSearch, params }) => {
    const input = {
      ...requestParamsForRoute(params),
      commit_oid: selectionSearch.commit,
      revision_id: selectionSearch.revision,
    }
    if (typeof window !== 'undefined') return null
    return loadChangesPage({ data: input })
  },
  pendingComponent: RequestChangesPending,
  component: RequestChangesRoute,
})

function RequestChangesRoute() {
  const page = requestRoute.useLoaderData()
  const changes = Route.useLoaderData()
  const params = Route.useParams()
  const search = Route.useSearch()
  const navigate = Route.useNavigate()
  const live = useRepoLayout()
  const { owner, repo, requestId } = params
  const requestParams = useMemo(
    () => requestParamsForRoute({ owner, repo, requestId }),
    [owner, repo, requestId],
  )
  const { userId, isLoaded } = useAuth()
  const scope = isLoaded ? repoResourceScope(live.repo, userId ?? null) : null
  const identity = scope
    ? requestChangesSelectionIdentity(scope, requestId, search.revision, search.commit)
    : null
  const load = useCallback(
    (signal: AbortSignal) => loadRevisions({
      data: {
        ...requestParams,
        commit_oid: search.commit,
        revision_id: search.revision,
      },
      signal,
    }),
    [requestParams, search.commit, search.revision],
  )
  const { initial, resource } = useRequestChangesResource({
    access: JSON.stringify(live.repo.access),
    identity,
    initial: changes,
    initialViewerId: page.account?.user?.id ?? null,
    load,
    viewerId: userId ?? null,
  })
  const revisions = resource.value ?? initial?.revisions ?? null
  const selection = revisions
    ? requestChangeSelection(revisions.revisions, revisions.review_revision_id, search)
    : null
  const pin = useMemo(
    () => requestRevisionPin(selection?.revision ?? null, selection?.commit ?? null, search.revision),
    [search.revision, selection?.commit, selection?.revision],
  )
  useEffect(() => {
    if (!pin || !scope || !revisions) return
    const pinnedIdentity = requestChangesSelectionIdentity(
      scope,
      requestId,
      pin.revision,
      pin.commit,
    )
    if (requestChangesResource.getSnapshot(pinnedIdentity).version === null) {
      requestChangesResource.write(pinnedIdentity, revisions)
    }
    void navigate({
      params,
      replace: true,
      resetScroll: false,
      search: (current) => ({ ...current, ...pin }),
      to: '/$owner/$repo/requests/$requestId/changes',
    })
  }, [navigate, params, pin, requestId, revisions, scope])

  if (!page.detail) return null

  if (!revisions && (!isLoaded || resource.refreshing)) return <RequestChangesPending />

  if (!revisions) {
    return (
      <EmptyState
        description="the discussion is still available. Try loading this revision again."
        action={
          <div className="flex flex-wrap justify-center gap-3">
            <Button
              disabled={resource.refreshing}
              onClick={resource.retry}
            >
              {resource.refreshing ? 'retrying changes…' : 'retry changes'}
            </Button>
            <Button asChild variant="secondary">
              <Link params={{ owner, repo, requestId }} to="/$owner/$repo/requests/$requestId">
                back to discussion
              </Link>
            </Button>
            <output className="sr-only">{resource.refreshing ? 'loading request changes' : 'changes could not load'}</output>
          </div>
        }
        icon={<GitCommit />}
        title="changes couldn't load"
      />
    )
  }

  return (
    <RequestChangesWorkbench
      accessScope={scope}
      audience={live.repo.access.can_read_private_files ? 'private' : 'public'}
      initialDiscussionReferences={initial?.discussionReferences ?? { commitKey: null, page: null }}
      loadDiff={loadDiffForView}
      loadDiscussions={loadDiscussionsForView}
      onSearchChange={(nextSearch) => {
        void navigate({
          params,
          replace: true,
          resetScroll: false,
          search: nextSearch,
          to: '/$owner/$repo/requests/$requestId/changes',
        })
      }}
      params={requestParams}
      repoId={live.repo.id}
      revisions={revisions}
      search={search}
    />
  )
}

function requestChangesSelectionSearch(search: unknown): RequestChangesSearch {
  if (!search || typeof search !== 'object') return {}
  const values = search as Record<string, unknown>
  return {
    commit: typeof values.commit === 'string' ? values.commit : undefined,
    revision: typeof values.revision === 'string' ? values.revision : undefined,
  }
}
