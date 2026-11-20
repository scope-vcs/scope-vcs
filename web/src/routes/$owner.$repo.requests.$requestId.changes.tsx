import {
  parseLoadRequestRevisionsInput,
  parseLoadRequestRevisionDiffInput,
  parseLoadDiscussionsInput,
  parseCreateDiscussionInput,
} from '@/api/request-inputs'
import {
  type LoadRequestRevisionDiffInput,
  loadRequestRevisionFileDiffForRequest,
  loadRequestRevisionsForRequest,
} from '@/api/requests'
import { EmptyState } from '@/components/empty-state'
import { Button } from '@/components/ui/button'
import {
  type CreateDiscussionInput,
  type LoadDiscussionsInput,
  createRequestDiscussionForRequest,
  loadRequestDiscussionsForRequest,
} from '@/features/requests/discussion/request-discussion-api'
import { RequestAttachmentProvider } from '@/features/requests/attachments/request-attachment-context'
import { requestAttachmentActions } from '@/routes/-request-attachment-actions'
import { loadDiscussionReferencePage } from '@/features/requests/changes/request-changes-discussion-references'
import { requestChangesResource, requestChangesSelectionIdentity, requestChangesQuery, retainPinnedRequestChanges, warmRequestChanges } from '@/features/requests/changes/request-changes-resource'
import { repoChangeGeneration } from '@/features/repo-detail/repo-resource'
import { repoSummaryIdentity, retainedRepoScope } from '@/features/repo-detail/repo-summary-resource'
import { requestStateAvailable, requestStateIdentity } from '@/features/requests/detail/request-state-resource'
import { useCachedResource } from '@/lib/use-cached-resource'
import {
  RequestChangesWorkbench,
  type RequestChangesSearch,
} from '@/features/requests/changes/request-changes-workbench'
import { RequestChangesBodyPending } from '@/features/requests/changes/request-changes-body-pending'
import { RequestChangesPending } from '@/features/requests/detail/request-page-pending'
import { RequestChangesScreen } from '@/features/requests/changes/request-changes-screen'
import {
  requestChangeSelection,
  requestRevisionPin,
} from '@/features/requests/changes/request-changes-model'
import { useRequestState } from '@/features/requests/detail/request-state-context'
import { requestParamsForRoute } from '@/features/requests/detail/request-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { parseRouteFilePathSearch } from '@/lib/route-file'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { Link, createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'
import { GitCommit } from 'lucide-react'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback, useEffect, useMemo } from 'react'

const loadRevisionDiff = createServerFn({ method: 'GET' })
  .validator(parseLoadRequestRevisionDiffInput)
  .handler(({ data }) => loadRequestRevisionFileDiffForRequest(data, getRequest().signal))

const loadRevisions = createServerFn({ method: 'GET' })
  .validator(parseLoadRequestRevisionsInput)
  .handler(({ data }) => loadRequestRevisionsForRequest(data))

const loadDiscussions = createServerFn({ method: 'GET' })
  .validator(parseLoadDiscussionsInput)
  .handler(({ data }) => loadDiscussionReferencePage(data, loadRequestDiscussionsForRequest))

const createDiscussion = createServerFn({ method: 'POST' })
  .validator(parseCreateDiscussionInput)
  .handler(({ data }) => createRequestDiscussionForRequest(data))

const createDiscussionForView = (data: CreateDiscussionInput) =>
  createDiscussion({ data })

const loadDiffForView = (
  data: LoadRequestRevisionDiffInput,
  signal?: AbortSignal,
) => loadRevisionDiff({ data, signal })
const loadDiscussionsForView = (data: LoadDiscussionsInput) =>
  loadDiscussions({ data })

export const Route = createFileRoute(
  '/$owner/$repo/requests/$requestId/changes',
)({
  validateSearch: (search: Record<string, unknown>): RequestChangesSearch => ({
    comparison: search.comparison === 'push' ? 'push' : search.comparison === 'commit' ? 'commit' : search.comparison === 'request' ? 'request' : search.commit ? 'commit' : undefined,
    commit: searchText(search.commit),
    path: parseRouteFilePathSearch(search.path),
    revision: searchText(search.revision),
  }),
  loaderDeps: ({ search }) => ({ comparison: search.comparison, commit: search.commit, revision: search.revision }),
  loader: async ({ deps, params, parentMatchPromise }) => {
    const query = requestChangesQuery(deps)
    const data = { ...requestParamsForRoute(params), ...query }
    const identity = (scope: string) =>
      requestChangesSelectionIdentity(scope, params.requestId, query.revision_id, query.commit_oid, query.comparison)
    const available = (scope: string) => requestStateAvailable(requestStateIdentity(scope, params.requestId))
    const known = retainedRepoScope(repoSummaryIdentity(params))
    if (known && requestChangesResource.isCurrent(identity(known))) return
    if (known && available(known)) {
      warmRequestChanges(identity(known), (signal) => loadRevisions({ data, signal }))
      return
    }
    const generation = known === null ? null : repoChangeGeneration(known)
    const revisions = loadRevisions({ data })
    void revisions.catch(() => {})
    const scope = (await parentMatchPromise.catch(() => null))?.loaderData?.scope
    if (scope && available(scope) && (generation === null || repoChangeGeneration(scope) === generation)) {
      warmRequestChanges(identity(scope), () => revisions)
    }
  },
  pendingComponent: RequestChangesPending,
  component: RequestChangesRoute,
})

function RequestChangesRoute() {
  const page = useRequestState()
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
  const query = useMemo(
    () => requestChangesQuery({ comparison: search.comparison, commit: search.commit, revision: search.revision }),
    [search.comparison, search.commit, search.revision],
  )
  const identity = scope
    ? requestChangesSelectionIdentity(scope, requestId, query.revision_id, query.commit_oid, query.comparison)
    : null
  const load = useCallback(
    (signal: AbortSignal) => loadRevisions({
      data: {
        ...requestParams,
        ...query,
      },
      signal,
    }),
    [requestParams, query],
  )
  const resource = useCachedResource({
    fallbackError: 'Request changes are unavailable.',
    identity,
    load,
    resource: requestChangesResource,
  })
  const revisions = resource.value
  const selection = revisions
    ? requestChangeSelection(revisions.revisions, revisions.review_revision_id, search)
    : null
  const pin = useMemo(
    () => requestRevisionPin(selection?.revision ?? null, query.comparison === 'commit' ? selection?.commit ?? null : null, search.revision),
    [search.revision, selection?.commit, selection?.revision, query.comparison],
  )
  useEffect(() => {
    if (!pin || !scope || !revisions) return
    retainPinnedRequestChanges(scope, requestId, { ...pin, comparison: query.comparison }, revisions)
    void navigate({
      params,
      replace: true,
      resetScroll: false,
      search: (current) => ({ ...current, ...pin }),
      to: '/$owner/$repo/requests/$requestId/changes',
    })
  }, [navigate, params, pin, requestId, revisions, scope, query.comparison])

  if (!page.state) return null

  return (
    <RequestChangesScreen
      params={params}
      revisions={revisions}
      selectedRevisionId={selection?.revision?.id ?? null}
      title={page.state.detail.request.title}
    >
      {revisions
        ? (
            <RequestAttachmentProvider
              actions={requestAttachmentActions}
              live={live}
              requestId={requestId}
              viewerId={page.viewerId ?? 'anonymous'}
            >
              <RequestChangesWorkbench
                accessScope={scope}
                canOpenDiscussion={page.state.detail.request.permissions.can_open_discussion}
                createDiscussion={createDiscussionForView}
                view={live.repo.access.view}
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
            </RequestAttachmentProvider>
          )
        : !isLoaded || resource.refreshing
          ? <RequestChangesBodyPending />
          : <RequestChangesUnavailable params={params} resource={resource} />}
    </RequestChangesScreen>
  )
}

function RequestChangesUnavailable({
  params,
  resource,
}: {
  params: { owner: string; repo: string; requestId: string }
  resource: { refreshing: boolean; retry: () => void }
}) {
  const { owner, repo, requestId } = params
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

function searchText(value: unknown) {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined
}
