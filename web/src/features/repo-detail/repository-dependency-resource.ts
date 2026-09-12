import type {
  RepositoryDependencyCheckResponse,
  RepoSummaryResponse,
} from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import { repoResourceScope } from './repo-resource-scope'

const DEPENDENCY_POLL_DELAY_MS = 5_000
const FAILED_DEPENDENCY_POLL_DELAY_MS = 30_000

type PollScheduler = (poll: () => void, delayMs: number) => () => void

export const repositoryDependencyResource = createRepositoryDependencyResource()

export function createRepositoryDependencyResource(
  schedulePoll: PollScheduler = browserPollScheduler,
) {
  const cache = createCachedResource<RepositoryDependencyCheckResponse>({
    maxEntries: 16,
    maxWeight: 16 * 1024 * 1024,
    weightOf: (value) => JSON.stringify(value).length * 2,
  })
  const polls = new Map<string, () => void>()

  function stopPoll(identity: string) {
    polls.get(identity)?.()
    polls.delete(identity)
  }

  function updatePoll(
    identity: string,
    value: RepositoryDependencyCheckResponse | null,
  ) {
    const retryFailedLoad = value === null && cache.getSnapshot(identity).error !== null
    const delayMs = retryFailedLoad || value?.status === 'Pending' || value?.status === 'Updating'
      ? DEPENDENCY_POLL_DELAY_MS
      : value?.status === 'Failed' ? FAILED_DEPENDENCY_POLL_DELAY_MS : null
    if (delayMs === null) {
      stopPoll(identity)
      return
    }
    if (polls.has(identity)) return
    polls.set(identity, schedulePoll(() => {
      polls.delete(identity)
      cache.invalidate(identity)
    }, delayMs))
  }

  async function ensure(
    identity: string,
    version: string,
    load: (signal: AbortSignal) => Promise<RepositoryDependencyCheckResponse>,
  ) {
    const value = await cache.ensure(identity, version, load)
    updatePoll(identity, value)
    return value
  }

  function invalidate(identity: string) {
    stopPoll(identity)
    cache.invalidate(identity)
  }

  return {
    subscribe: cache.subscribe,
    getSnapshot: cache.getSnapshot,
    getServerSnapshot: cache.getServerSnapshot,
    peek: cache.peek,
    ensure,
    invalidate,
    write(identity: string, value: RepositoryDependencyCheckResponse, version = '') {
      cache.write(identity, value, version)
      updatePoll(identity, value)
    },
    clear() {
      for (const identity of polls.keys()) stopPoll(identity)
      cache.clear()
    },
  }
}

export function repositoryDependencyIdentity(
  repo: RepoSummaryResponse,
  viewerId: string | null,
) {
  return repo.access.actor === 'Public' || viewerId === null
    ? null
    : repoResourceScope(repo, viewerId)
}

function browserPollScheduler(poll: () => void, delayMs: number) {
  const timer = globalThis.setTimeout(poll, delayMs)
  const nodeTimer = timer as { unref?: () => void }
  nodeTimer.unref?.()
  return () => globalThis.clearTimeout(timer)
}
