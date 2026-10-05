import type { RepoRunHistoryInput } from '@/api/types'
import type { GitHubWorkflowRunListResponse, RepositoryRunWorkflowListResponse, RepositoryRunHistoryPageResponse } from '@/api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import { runResourceNeedsRecovery } from './run-resource'
import { mergeRunHistory, reloadRunHistoryPages } from './run-history-model'

export type RunPageResources = {
  kind: 'native'
  githubConfigured: boolean
  history: RepositoryRunHistoryPageResponse
  workflows: RepositoryRunWorkflowListResponse
  workflowsError: string | null
} | { kind: 'github'; github: GitHubWorkflowRunListResponse }

export type RunPageHandoff = { scope: string; resources: RunPageResources | null }

export type RetainedRunHistory = {
  page?: RunPageResources | null
  updatedAt?: number
  history: RepositoryRunHistoryPageResponse | null
  pageCount: number
}

export const runHistoryResource = createCachedResource<RetainedRunHistory>({
  maxEntries: 12,
  maxWeight: 4 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function runHistoryCacheKey(scope: string, workflow?: string) {
  return JSON.stringify([scope, workflow ?? null])
}

export function initializeRunHistory(key: string, history: RepositoryRunHistoryPageResponse | null) {
  runHistoryResource.seed(key, { history, pageCount: 1, updatedAt: Date.now() }, '0')
}

type HistoryRequest = {
  key: string
  input: RepoRunHistoryInput
  loadHistory: (input: RepoRunHistoryInput, signal?: AbortSignal) => Promise<RepositoryRunHistoryPageResponse | null>
}

export async function refreshRunHistory({ key, input, loadHistory, loadPage }: HistoryRequest & { loadPage: RunPageLoader }, recovery = false): Promise<void> {
  if (recovery && !runResourceNeedsRecovery(runHistoryResource, key)) return
  const snapshot = runHistoryResource.getSnapshot(key)
  const current = snapshot.value
  if (!current?.history) return
  const load = (signal: AbortSignal) => loadRunPageSnapshot({ key, input, loadHistory, loadPage, signal })
  if (snapshot.pending) {
    if (snapshot.version === 'refresh') {
      await runHistoryResource.load(key, 'refresh', load)
      return
    }
    // A live change during pagination must reconcile the newly loaded depth.
    await runHistoryResource.ensure(key, snapshot.version!, load)
    return refreshRunHistory({ key, input, loadHistory, loadPage })
  }
  runHistoryResource.invalidate(key)
  await runHistoryResource.load(key, 'refresh', load)
}

export async function loadMoreRunHistory({ key, input, loadHistory }: HistoryRequest): Promise<void> {
  const snapshot = runHistoryResource.getSnapshot(key)
  const current = snapshot.value
  if (snapshot.pending || !current?.history?.next_cursor) return
  const after = current.history.next_cursor
  runHistoryResource.invalidate(key)
  await runHistoryResource.ensure(key, 'more', async (signal) => {
    const next = await loadHistory({ ...input, after }, AbortSignal.any([signal, AbortSignal.timeout(15_000)]))
    return {
      ...current,
      history: next ? {
        next_cursor: next.next_cursor,
        runs: mergeRunHistory(current.history!.runs, next.runs),
      } : null,
      pageCount: current.pageCount + (next ? 1 : 0),
    }
  })
}

export function runPageSnapshot(page: RunPageResources | null): RetainedRunHistory {
  const history = page?.kind === 'native' ? page.history : null
  return { page, history, pageCount: 1, updatedAt: Date.now() }
}

export type RunPageLoader = (input: RepoRunHistoryInput, signal?: AbortSignal) => Promise<RunPageResources | null>

export async function loadRunPageSnapshot({ key, input, loadPage, loadHistory, signal }: HistoryRequest & {
  loadPage: RunPageLoader
  signal: AbortSignal
}): Promise<RetainedRunHistory> {
  const current = runHistoryResource.peek(key)
  const page = await loadPage(input, AbortSignal.any([signal, AbortSignal.timeout(15_000)]))
  const next = runPageSnapshot(page)
  if (page?.kind !== 'native' || !current?.history || current.pageCount === 1) return next
  const history = await reloadRunHistoryPages(current.pageCount, (after) => after
    ? loadHistory({ ...input, after }, AbortSignal.any([signal, AbortSignal.timeout(15_000)]))
    : Promise.resolve(page.history))
  return { ...next, history, pageCount: current.pageCount }
}

const pendingPaginationInvalidations = new WeakSet<object>()

export function invalidateRunHistoryScope(scope: string, recovery = false) {
  const prefix = `${JSON.stringify([scope]).slice(0, -1)},`
  runHistoryResource.invalidateMatching((key) => {
    if (!key.startsWith(prefix) || recovery && !runResourceNeedsRecovery(runHistoryResource, key)) return false
    const snapshot = runHistoryResource.getSnapshot(key)
    if (!snapshot.pending || snapshot.version !== 'more') return true
    // Let the older page join retained history before reconciling its full depth.
    if (!pendingPaginationInvalidations.has(snapshot)) {
      pendingPaginationInvalidations.add(snapshot)
      const unsubscribe = runHistoryResource.subscribe(key, () => {
        if (runHistoryResource.getSnapshot(key).pending) return
        unsubscribe()
        if (runHistoryResource.peek(key)) runHistoryResource.invalidate(key)
      })
    }
    return false
  })
}
