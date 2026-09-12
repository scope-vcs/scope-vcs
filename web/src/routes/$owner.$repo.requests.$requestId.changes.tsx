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
import {
  forgetPinnedChangesReplay,
  rememberPinnedChangesReplay,
  takePinnedChangesReplay,
} from '@/features/requests/request-changes-replay'
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
import { Link, createFileRoute, getRouteApi, useRouter } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'
import { GitCommit } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'

type ChangesPage = Awaited<ReturnType<typeof loadChangesPage>>
type ChangesLoaderData = ChangesPage & {
  pin: RequestChangesSearch | null
}

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
    const replay = takePinnedChangesReplay<ChangesLoaderData>(input)
    if (replay) return replay
    const page = await loadChangesPage({ data: input })
    return pinChangesPage(page, selectionSearch)
  },
  pendingComponent: RequestChangesPending,
  component: RequestChangesRoute,
})

function RequestChangesRoute() {
  const router = useRouter()
  const [retrying, setRetrying] = useState(false)
  const page = requestRoute.useLoaderData()
  const changes = Route.useLoaderData()
  const matchId = Route.useMatch().id
  const params = Route.useParams()
  const search = Route.useSearch()
  const navigate = Route.useNavigate()
  const live = useRepoLayout()
  const { owner, repo, requestId } = params
  const requestParams = useMemo(
    () => requestParamsForRoute({ owner, repo, requestId }),
    [owner, repo, requestId],
  )
  useEffect(() => {
    if (!changes.pin || search.revision) return
    const replay = rememberPinnedChangesReplay(
      {
        ...requestParams,
        commit_oid: changes.pin.commit,
        revision_id: changes.pin.revision,
      },
      changes,
    )
    void navigate({
      params,
      replace: true,
      resetScroll: false,
      search: (current) => ({ ...current, ...changes.pin }),
      to: '/$owner/$repo/requests/$requestId/changes',
    }).then(
      () => forgetPinnedChangesReplay(replay),
      () => forgetPinnedChangesReplay(replay),
    )
  }, [changes, navigate, params, requestParams, search.revision])

  if (!page.detail) return null

  if (!changes.revisions) {
    return (
      <EmptyState
        description="the discussion is still available. Try loading this revision again."
        action={
          <div className="flex flex-wrap justify-center gap-3">
            <Button
              disabled={retrying}
              onClick={() => {
                setRetrying(true)
                void loadChangesPage({
                  data: {
                    ...requestParams,
                    commit_oid: search.commit,
                    revision_id: search.revision,
                  },
                })
                  .then((result) => {
                    router.updateMatch(matchId, (match) => ({
                      ...match,
                      loaderData: pinChangesPage(result, search),
                    }))
                  })
                  .catch((error: unknown) => console.error('Retrying request changes failed', error))
                  .finally(() => setRetrying(false))
              }}
            >
              {retrying ? 'retrying changes…' : 'retry changes'}
            </Button>
            <Button asChild variant="secondary">
              <Link params={{ owner, repo, requestId }} to="/$owner/$repo/requests/$requestId">
                back to discussion
              </Link>
            </Button>
            <output className="sr-only">{retrying ? 'loading request changes' : 'changes could not load'}</output>
          </div>
        }
        icon={<GitCommit />}
        title="changes couldn't load"
      />
    )
  }

  return (
    <RequestChangesWorkbench
      accessScope={repoResourceScope(live.repo, page.account?.user?.id ?? null)}
      audience={live.repo.access.can_read_private_files ? 'private' : 'public'}
      initialDiscussionReferences={changes.discussionReferences}
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
      revisions={changes.revisions}
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

function pinChangesPage(page: ChangesPage, search: RequestChangesSearch): ChangesLoaderData {
  const { revisions } = page
  if (!revisions) return { ...page, pin: null }
  const selection = requestChangeSelection(
    revisions.revisions,
    revisions.review_revision_id,
    search,
  )
  return {
    ...page,
    pin: requestRevisionPin(selection.revision, selection.commit, search.revision),
  }
}
