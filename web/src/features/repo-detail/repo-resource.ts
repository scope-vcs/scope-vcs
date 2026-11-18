import type { RepoChangeEvent } from '../../api/types.generated'
import type { BoundedCacheOptions } from '../../lib/bounded-cache'
import { createCachedResource, type CachedResourceStore } from '../../lib/cached-resource'

const REPO_TOPICS = [
  'summary',
  'repository',
  'content',
  'dependencies',
  'requestList',
  'request',
  'requestHead',
  'requestTimeline',
  'requestAttachments',
  'checks',
  'workflowRun',
] as const

export type RepoTopic = (typeof REPO_TOPICS)[number]

export type RepoChange = {
  topic: RepoTopic
  requestId?: string
  runId?: number
  version?: number
}

type RepoResync = {
  versioned?: boolean
}

type RepoResourceOptions<T extends object> = BoundedCacheOptions<T> & {
  invalidatedBy: readonly RepoTopic[]
  affectedBy?: (change: RepoChange, key: string[], entry: RepoResourceEntry<T>) => boolean
  onInvalidate?: (identity: string) => void
  keyIn?: (scope: string, entry: RepoResourceEntry<T>) => string[] | null
  supersedes?: readonly RepoTopic[]
  coalesceInvalidations?: boolean
}

type RepoResourceEntry<T extends object> = {
  identity: string
  snapshot: ReturnType<CachedResourceStore<T>['getSnapshot']>
}

type RegisteredRepoResource = {
  invalidateFor: (scope: string, changes: RepoChange[]) => boolean
  supersedes: readonly RepoTopic[]
}

const repoResources: RegisteredRepoResource[] = []

export function repoResourceKey(scope: string, ...parts: Array<string | number>) {
  return [scope, ...parts].join('\0')
}

export function repoResourceKeyParts(scope: string, identity: string) {
  if (identity === scope) return []
  return identity.startsWith(`${scope}\0`) ? identity.slice(scope.length + 1).split('\0') : null
}

export function createRepoResource<T extends object>({
  invalidatedBy,
  affectedBy = () => true,
  onInvalidate,
  keyIn = (scope, { identity }) => repoResourceKeyParts(scope, identity),
  supersedes = [],
  ...options
}: RepoResourceOptions<T>) {
  const cache = createCachedResource<T>(options)
  const resource = {
    ...cache,
    supersedes,
    invalidateFor(scope: string, changes: RepoChange[]) {
      const relevant = changes.filter((change) => invalidatedBy.includes(change.topic))
      let invalidated = false
      if (relevant.length === 0) return invalidated
      cache.invalidateMatching((identity) => {
        const entry = { identity, snapshot: cache.getSnapshot(identity) }
        const key = keyIn(scope, entry)
        if (!key || !relevant.some((change) => affectedBy(change, key, entry))) return false
        onInvalidate?.(identity)
        invalidated = true
        return true
      })
      return invalidated
    },
  }
  repoResources.push(resource)
  return resource
}

export function affectsRequestKey(change: RepoChange, key: string[]) {
  return change.requestId === undefined || key[0] === change.requestId
}

export function invalidateRepoResources(scope: string, changes: RepoChange[]) {
  let remaining = changes
  for (const resource of repoResources) {
    if (resource.supersedes.length > 0 && resource.invalidateFor(scope, changes)) {
      remaining = remaining.filter(({ topic }) => !resource.supersedes.includes(topic))
    }
  }
  for (const resource of repoResources) {
    if (resource.supersedes.length === 0) resource.invalidateFor(scope, remaining)
  }
}

export function resyncTopics({ versioned = false }: RepoResync = {}): RepoChange[] {
  return everyTopic().filter(({ topic }) =>
    !versioned || (topic !== 'content' && topic !== 'dependencies' && topic !== 'requestHead'))
}

export function topicsOf(event: RepoChangeEvent, { versioned = false }: RepoResync = {}): RepoChange[] {
  const { kind } = event
  const summary: RepoChange = { topic: 'summary', ...event.version === 0 ? {} : { version: event.version } }
  const requestList: RepoChange = { topic: 'requestList' }
  if (typeof kind === 'string') {
    switch (kind) {
      case 'Connected':
      case 'Lagged':
        return resyncTopics({ versioned })
      case 'DependenciesChanged':
        return [{ topic: 'dependencies' }, summary]
      case 'GitHubWorkflowRunsChanged':
        return [{ topic: 'checks' }, requestList]
      default:
        return unhandledKind(kind)
    }
  }
  if ('RepositoryChanged' in kind) return everyTopic().map((change) => change.topic === 'summary' ? summary : change)
  if ('GitHubWorkflowRunChanged' in kind) {
    return [{ topic: 'workflowRun', runId: kind.GitHubWorkflowRunChanged.github_run_id }]
  }
  if ('RequestStateChanged' in kind) {
    const requestId = kind.RequestStateChanged.request_id
    return [{ topic: 'request', requestId }, { topic: 'requestHead', requestId }, requestList, summary]
  }
  if ('RequestTimelineChanged' in kind) {
    return [{ topic: 'requestTimeline', requestId: kind.RequestTimelineChanged.request_id }, requestList]
  }
  if ('RequestAttachmentChanged' in kind) {
    return [{ topic: 'requestAttachments', requestId: kind.RequestAttachmentChanged.request_id }, summary]
  }
  return unhandledKind(kind)
}

function everyTopic(): RepoChange[] {
  return REPO_TOPICS.map((topic) => ({ topic }))
}

function unhandledKind(kind: never): never {
  throw new Error(`Unhandled repository change: ${JSON.stringify(kind)}`)
}
