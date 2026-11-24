import type { RequestBacklogQuery } from '@/api/request-backlog-input'
import {
  requestBacklogQuery,
  requestBacklogRouteSearch,
  type RequestBacklogRouteSearch,
} from '@/features/requests/backlog/request-backlog-model'
import { RequestBacklogPage } from '@/features/requests/backlog/request-backlog-page'
import { createFileRoute } from '@tanstack/react-router'
import { useCallback, useMemo } from 'react'

export const Route = createFileRoute('/$owner/$repo/requests/backlog')({
  validateSearch: (search: Record<string, unknown>): RequestBacklogRouteSearch =>
    requestBacklogRouteSearch(requestBacklogQuery(search)),
  component: RequestBacklogRoute,
})

function RequestBacklogRoute() {
  const { owner, repo } = Route.useParams()
  const { q, sort, working } = Route.useSearch()
  const navigate = Route.useNavigate()
  const params = useMemo(() => ({ owner, repo }), [owner, repo])
  const query = useMemo(() => requestBacklogQuery({ q, sort, working }), [q, sort, working])
  const changeQuery = useCallback(
    (next: RequestBacklogQuery) => void navigate({
      replace: true,
      resetScroll: false,
      search: (current) => ({ ...current, ...requestBacklogRouteSearch(next) }),
    }),
    [navigate],
  )
  return <RequestBacklogPage onQueryChange={changeQuery} params={params} query={query} />
}
