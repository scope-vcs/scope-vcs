import type { RequestBacklogQuery } from '@/api/request-backlog-input'
import { REQUEST_BACKLOG_SORTS, REQUEST_BACKLOG_WORKING } from '@/api/request-backlog-input'
import type { RepoParams } from '@/api/types'
import type { RequestBacklogItemResponse, RequestCloseReason } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { Select } from '@/components/ui/select'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { useHydrated } from '@/lib/use-hydrated'
import { useUnixClock } from '@/lib/use-unix-clock'
import { requestBacklogPageLoader } from '@/routes/-request-workspace-actions'
import { runRequestAction } from '@/routes/-request-state-actions'
import { useAuth } from '@clerk/tanstack-react-start'
import { Link } from '@tanstack/react-router'
import { ArrowLeft, LoaderCircle, Search } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useRepoLayout } from '../../repo-detail/repo-layout-context'
import { invalidateRepoResources } from '../../repo-detail/repo-resource'
import { repoResourceScope } from '../../repo-detail/repo-resource-scope'
import { useViewingAs } from '../../repo-detail/use-viewing-as'
import { RequestCloseReasonField } from '../detail/request-close-reason-field'
import { RequestConfirmDialog } from '../detail/request-confirm-dialog'
import { useRequestWorkspace } from '../workspace/request-workspace-context'
import { requestAgeLabel } from '../workspace/request-workspace-model'
import {
  closeBacklogRequests,
  REQUEST_BACKLOG_NAME,
  REQUEST_BACKLOG_SORT_LABELS,
  REQUEST_BACKLOG_WORKING_LABELS,
  requestBacklogCountLabel,
  requestBacklogRepliesLabel,
  requestBacklogSource,
  type BacklogCloseFailures,
} from './request-backlog-model'
import { loadMoreRequestBacklog, requestBacklogIdentity } from './request-backlog-resource'
import { useRequestBacklog } from './use-request-backlog'
import './request-backlog.css'

const SEARCH_DEBOUNCE_MS = 250
const NO_PICKS: Picks = { identity: null, ids: new Set(), failures: new Map() }

type Picks = {
  identity: string | null
  ids: ReadonlySet<string>
  failures: BacklogCloseFailures
}

export function RequestBacklogPage({
  onQueryChange,
  params,
  query,
}: {
  onQueryChange: (query: RequestBacklogQuery) => void
  params: RepoParams
  query: RequestBacklogQuery
}) {
  const { isLoaded, userId } = useAuth()
  const { repo } = useRepoLayout()
  const { view } = useViewingAs()
  const backlogTotal = useRequestWorkspace()?.backlogTotal ?? null
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const identity = scope && requestBacklogIdentity(scope, view, query)
  const load = useMemo(
    () => requestBacklogPageLoader({ owner: params.owner, repo: params.repo }, view, query),
    [params.owner, params.repo, query, view],
  )
  const backlog = useRequestBacklog(identity, String(repo.change_version), load)
  const [picks, setPicks] = useState(NO_PICKS)
  const [confirming, setConfirming] = useState(false)
  const [closing, setClosing] = useState(false)
  const [reason, setReason] = useState<RequestCloseReason | null>(null)
  const [loadingMore, setLoadingMore] = useState(false)
  const closeButton = useRef<HTMLButtonElement>(null)

  const rows = backlog.value?.requests ?? []
  const closable = rows.filter((row) => row.can_close)
  const current = picks.identity === identity ? picks : NO_PICKS
  const picked = closable.filter((row) => current.ids.has(row.id))
  const failed = rows.filter((row) => current.failures.has(row.id))
  const everyClosablePicked = closable.length > 0 && picked.length === closable.length

  function pick(ids: Iterable<string>) {
    const next = new Set(ids)
    setPicks({
      identity,
      ids: next,
      failures: new Map([...current.failures].filter(([id]) => next.has(id))),
    })
  }

  function toggle(row: RequestBacklogItemResponse) {
    const next = new Set(current.ids)
    if (!next.delete(row.id)) next.add(row.id)
    pick(next)
  }

  async function closePicked() {
    if (!reason || !scope) return false
    setClosing(true)
    const failures = await closeBacklogRequests(
      picked.map((row) => row.id),
      (request_id) => runRequestAction({ data: { ...params, request_id, action: 'close', reason } }),
    )
    setPicks({ identity, ids: new Set(failures.keys()), failures })
    invalidateRepoResources(scope, [{ topic: 'requestList' }])
    setClosing(false)
    return true
  }

  async function loadMore() {
    if (!identity) return
    setLoadingMore(true)
    await loadMoreRequestBacklog(identity, load).catch(() => {})
    setLoadingMore(false)
  }

  return (
    <div className="request-backlog" data-selectable={closable.length > 0 || undefined}>
      <header className="px-(--app-inset) pt-3 min-[701px]:pt-6">
        <nav aria-label={`${REQUEST_BACKLOG_NAME} navigation`} className="mb-3 min-[701px]:hidden">
          <Link
            className="flex w-fit items-center gap-1.5 rounded py-1 text-xs text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
            params={params}
            to="/$owner/$repo/requests"
          >
            <ArrowLeft aria-hidden="true" className="size-3.5" /> Requests
          </Link>
        </nav>
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
          <h1 className="text-[28px] font-semibold leading-[1.15] tracking-[-0.025em] sm:text-[30px]">
            {REQUEST_BACKLOG_NAME}
          </h1>
          {backlogTotal !== null && (
            <p className="text-sm text-muted-foreground">
              {requestBacklogCountLabel(backlogTotal)} not in review yet
            </p>
          )}
        </div>
      </header>
      <div className="request-backlog-tools">
        <BacklogSearch onChange={(search) => onQueryChange({ ...query, search })} value={query.search} />
        <Select
          aria-label="Filter by who is working"
          className="request-backlog-select"
          onChange={(event) => onQueryChange({
            ...query,
            working: REQUEST_BACKLOG_WORKING.find((working) => working === event.target.value) ?? query.working,
          })}
          value={query.working}
        >
          {REQUEST_BACKLOG_WORKING.filter((working) => working !== 'me' || userId).map((working) => (
            <option key={working} value={working}>{REQUEST_BACKLOG_WORKING_LABELS[working]}</option>
          ))}
        </Select>
        <Select
          aria-label="Sort"
          className="request-backlog-select"
          onChange={(event) => onQueryChange({
            ...query,
            sort: REQUEST_BACKLOG_SORTS.find((sort) => sort === event.target.value) ?? query.sort,
          })}
          value={query.sort}
        >
          {REQUEST_BACKLOG_SORTS.map((sort) => (
            <option key={sort} value={sort}>{REQUEST_BACKLOG_SORT_LABELS[sort]}</option>
          ))}
        </Select>
        {picked.length > 0 && (
          <p className="request-backlog-picked">
            <span aria-live="polite">{picked.length} selected</span>
            <Button
              disabled={closing}
              onClick={() => {
                setReason(null)
                setConfirming(true)
              }}
              ref={closeButton}
              size="sm"
              type="button"
              variant="secondary"
            >
              Close
            </Button>
          </p>
        )}
      </div>
      {failed.length > 0 && (
        <p className="px-(--app-inset) pb-3 text-[13px] text-danger-strong" role="alert">
          {requestBacklogCountLabel(failed.length)} did not close and{' '}
          {failed.length === 1 ? 'is' : 'are'} still selected.
        </p>
      )}
      {backlog.error && (
        <p className="flex flex-wrap items-center gap-2 px-(--app-inset) pb-3 text-[13px] text-danger-strong" role="alert">
          {backlog.error}
          <Button onClick={backlog.retry} size="sm" type="button" variant="secondary">Try again</Button>
        </p>
      )}
      <ul aria-busy={backlog.refreshing} className="request-backlog-list">
        <li className="request-backlog-row request-backlog-columns" role="presentation">
          {closable.length > 0 && (
            <input
              aria-label="Select every loaded request"
              checked={everyClosablePicked}
              className="request-backlog-check"
              onChange={() => pick(everyClosablePicked ? [] : closable.map((row) => row.id))}
              ref={(input) => {
                if (input) input.indeterminate = picked.length > 0 && !everyClosablePicked
              }}
              type="checkbox"
            />
          )}
          <span aria-hidden="true" className="request-backlog-select-all">Select all</span>
          <span aria-hidden="true">Request</span>
          <span aria-hidden="true">Working on it</span>
          <span aria-hidden="true">Opened</span>
          <span aria-hidden="true" className="request-backlog-replies">Replies</span>
        </li>
        {rows.map((row) => (
          <BacklogRow
            failure={current.failures.get(row.id)}
            key={row.id}
            onToggle={() => toggle(row)}
            params={params}
            picked={current.ids.has(row.id)}
            row={row}
            selectable={closable.length > 0}
          />
        ))}
        {!backlog.value && !backlog.error && <BacklogSkeleton />}
      </ul>
      {backlog.value && rows.length === 0 && (
        <p className="px-(--app-inset) py-10 text-center text-sm text-muted-foreground">
          {query.search || query.working !== 'anyone'
            ? 'No requests match this search and filter.'
            : 'No requests are waiting here.'}
        </p>
      )}
      {backlog.value && rows.length > 0 && (
        <p className="request-backlog-more">
          Showing {rows.length} of {backlog.value.total}.
          {backlog.value.next_cursor && (
            <Button disabled={loadingMore} onClick={() => void loadMore()} size="sm" type="button" variant="ghost">
              {loadingMore ? <LoaderCircle className="animate-spin" /> : null}
              Load more
            </Button>
          )}
        </p>
      )}
      <RequestConfirmDialog
        confirmDisabled={reason === null}
        confirmLabel={`Close ${requestBacklogCountLabel(picked.length)}`}
        destructive
        onConfirm={closePicked}
        onOpenChange={setConfirming}
        open={confirming}
        pending={closing}
        returnFocus={closeButton}
        title={`Close ${requestBacklogCountLabel(picked.length)}?`}
      >
        <p>
          {picked.length === 1
            ? 'This request will close and remain in request history.'
            : 'These requests will close and remain in request history.'}
        </p>
        <RequestCloseReasonField
          legend={picked.length === 1 ? 'Why is it closing?' : 'Why are they closing?'}
          onChange={setReason}
          value={reason}
        />
      </RequestConfirmDialog>
    </div>
  )
}

function BacklogSearch({ onChange, value }: { onChange: (search: string) => void; value: string }) {
  const [draft, setDraft] = useState({ applied: value, text: value })
  if (draft.applied !== value) {
    setDraft({ applied: value, text: draft.text.trim() === value ? draft.text : value })
  }
  const next = draft.text.trim()
  useEffect(() => {
    if (next === value) return
    const timer = window.setTimeout(() => onChange(next), SEARCH_DEBOUNCE_MS)
    return () => window.clearTimeout(timer)
  }, [next, onChange, value])

  return (
    <label className="request-backlog-search">
      <Search aria-hidden="true" />
      <span className="sr-only">Search the {REQUEST_BACKLOG_NAME.toLowerCase()}</span>
      <input
        onChange={(event) => setDraft({ applied: value, text: event.target.value })}
        placeholder="Search titles, text, or a GitHub number"
        type="search"
        value={draft.text}
      />
    </label>
  )
}

function BacklogRow({
  failure,
  onToggle,
  params,
  picked,
  row,
  selectable,
}: {
  failure: string | undefined
  onToggle: () => void
  params: RepoParams
  picked: boolean
  row: RequestBacklogItemResponse
  selectable: boolean
}) {
  const hydrated = useHydrated()
  const nowUnix = useUnixClock()
  return (
    <li className="request-backlog-row" data-picked={picked || undefined}>
      {selectable && (
        row.can_close ? (
          <input
            aria-label={`Select ${row.title}`}
            checked={picked}
            className="request-backlog-check"
            onChange={onToggle}
            type="checkbox"
          />
        ) : <span />
      )}
      <span className="request-backlog-name">
        <Link
          className="request-backlog-link"
          params={{ ...params, requestId: row.id }}
          to="/$owner/$repo/requests/$requestId"
        >
          {row.title}
        </Link>
        <span className="request-backlog-source">{requestBacklogSource(row)}</span>
        {failure && <span className="request-backlog-failure">Did not close: {failure}</span>}
      </span>
      <span className="request-backlog-facts">
        <span className="request-backlog-worker">
          {row.worker && (
            <>
              <span className="request-backlog-fact-label">Working on it: </span>
              {row.worker.handle}
            </>
          )}
        </span>
        <time
          className="request-backlog-opened"
          dateTime={new Date(row.asked_at_unix * 1_000).toISOString()}
          suppressHydrationWarning
        >
          <span className="request-backlog-fact-label">Opened </span>
          {requestAgeLabel(row.asked_at_unix, nowUnix, hydrated)}
        </time>
        <span className="request-backlog-replies">
          <span aria-hidden="true" className="request-backlog-replies-count">{row.message_count}</span>
          <span className="request-backlog-fact-label">{requestBacklogRepliesLabel(row.message_count)}</span>
        </span>
      </span>
    </li>
  )
}

function BacklogSkeleton() {
  return (
    <>
      {(['long', 'xlong', 'medium', 'short'] as const).map((length) => (
        <li aria-hidden="true" className="request-backlog-row" key={length}>
          <span className="request-backlog-name">
            <TextSkeleton length={length} />
            <TextSkeleton length="short" size="meta" />
          </span>
        </li>
      ))}
    </>
  )
}
