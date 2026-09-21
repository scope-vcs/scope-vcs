import { createCachedResource } from '../../lib/cached-resource'
import {
  renderRequestMermaid,
  type MermaidRenderInput,
  type MermaidRenderResult,
} from './request-mermaid-renderer'
import { createRequestAttachmentScopeTracker } from './request-attachment-scope-tracker'

export type RequestMermaidInput = {
  accessScope: string
  source: string
  theme: 'light' | 'dark'
}

export type RequestMermaidResult = MermaidRenderResult

export type RequestMermaidPriority = 0 | 1

type RenderRequestMermaid = (input: MermaidRenderInput) => Promise<MermaidRenderResult>

type Demand = {
  attempted: boolean
  input: RequestMermaidInput
  leases: Map<symbol, RequestMermaidPriority>
  retryPriority: RequestMermaidPriority | null
  sequence: number
}

type RenderTask = {
  demand: Demand
  deferred: ReturnType<typeof deferred<RequestMermaidResult>>
  identity: string
  input: RequestMermaidInput
  priority: RequestMermaidPriority
  sequence: number
  state: 'queued' | 'started'
  retryAfter: boolean
}

const RENDER_VERSION = '1'
const MAX_CACHE_ENTRIES = 64
const MAX_CACHE_WEIGHT = 4 * 1024 * 1024
const MAX_QUEUED_RENDERS = 64

export function requestMermaidIdentity(input: RequestMermaidInput) {
  return `${input.accessScope}\0${input.theme}\0${RENDER_VERSION}\0${input.source}`
}

export function createRequestMermaidResourceManager({
  render,
  yieldToBrowser = yieldBrowserTask,
  maxEntries = MAX_CACHE_ENTRIES,
  maxWeight = MAX_CACHE_WEIGHT,
  maxQueued = MAX_QUEUED_RENDERS,
}: {
  render: RenderRequestMermaid
  yieldToBrowser?: () => Promise<void>
  maxEntries?: number
  maxWeight?: number
  maxQueued?: number
}) {
  const retainedWeights = new WeakMap<RequestMermaidResult, number>()
  const resource = createCachedResource<RequestMermaidResult>({
    maxEntries,
    maxWeight,
    weightOf: (value) => retainedWeights.get(value) ?? value.svg.length * 2,
  })
  const demands = new Map<string, Demand>()
  const tasks = new Map<string, RenderTask>()
  let running: RenderTask | null = null
  let nextSequence = 0

  const scopeTracker = createRequestAttachmentScopeTracker({
    maxOwners: 16,
    removePreviousScope: removeScope,
  })

  function effectivePriority(demand: Demand) {
    let priority: RequestMermaidPriority = demand.retryPriority ?? 1
    for (const leasePriority of demand.leases.values()) {
      priority = Math.min(priority, leasePriority) as RequestMermaidPriority
    }
    return priority
  }

  function isReady(identity: string) {
    const snapshot = resource.getSnapshot(identity)
    return snapshot.value !== null
      && snapshot.version === RENDER_VERSION
      && !snapshot.stale
  }

  function canQueue(identity: string) {
    const snapshot = resource.getSnapshot(identity)
    const demand = demands.get(identity)
    return demand !== undefined
      && !demand.attempted
      && !isReady(identity)
      && !snapshot.pending
      && snapshot.error === null
  }

  function cancelQueued(task: RenderTask, discard: boolean) {
    if (task.state !== 'queued' || tasks.get(task.identity) !== task) return
    tasks.delete(task.identity)
    task.deferred.reject(new Error('Mermaid render was canceled.'))
    if (discard) {
      resource.removeMatching((identity) => identity === task.identity)
    } else {
      resource.invalidate(task.identity)
    }
  }

  function createTask(identity: string, demand: Demand) {
    const task: RenderTask = {
      demand,
      deferred: deferred<RequestMermaidResult>(),
      identity,
      input: demand.input,
      priority: effectivePriority(demand),
      sequence: demand.sequence,
      state: 'queued',
      retryAfter: false,
    }
    demand.retryPriority = null
    tasks.set(identity, task)
    void resource.ensure(identity, RENDER_VERSION, () => task.deferred.promise)
  }

  function reconcileQueue() {
    const candidates = [...demands.entries()]
      .filter(([identity]) => {
        const task = tasks.get(identity)
        return task?.state === 'queued' || (!task && canQueue(identity))
      })
      .sort((left, right) => {
        const priority = effectivePriority(left[1]) - effectivePriority(right[1])
        return priority || left[1].sequence - right[1].sequence
      })

    const selected = new Set(candidates.slice(0, maxQueued).map(([identity]) => identity))
    for (const task of tasks.values()) {
      if (task.state === 'queued' && !selected.has(task.identity)) cancelQueued(task, false)
    }
    for (const [identity, demand] of candidates.slice(0, maxQueued)) {
      const existing = tasks.get(identity)
      if (existing) {
        existing.priority = effectivePriority(demand)
      } else {
        createTask(identity, demand)
      }
    }
    pump()
  }

  function pump() {
    if (running) return
    const next = [...tasks.values()]
      .filter((task) => task.state === 'queued')
      .sort((left, right) => left.priority - right.priority || left.sequence - right.sequence)[0]
    if (!next) return
    next.state = 'started'
    running = next
    void run(next)
  }

  async function run(task: RenderTask) {
    try {
      const result = await render({ source: task.input.source, theme: task.input.theme })
      retainedWeights.set(result, (task.input.source.length + result.svg.length) * 2)
      task.deferred.resolve(result)
    } catch (error) {
      task.deferred.reject(error)
    } finally {
      const demand = demands.get(task.identity)
      if (demand === task.demand) demand.attempted = !task.retryAfter
      if (tasks.get(task.identity) === task) tasks.delete(task.identity)
      await yieldToBrowser().catch(() => {})
      if (running === task) running = null
      reconcileQueue()
    }
  }

  function acquire(input: RequestMermaidInput, priority: RequestMermaidPriority) {
    const identity = requestMermaidIdentity(input)
    // Reopening a retained result is a cache read, even when no render is needed.
    resource.read(identity)
    const lease = Symbol(identity)
    let demand = demands.get(identity)
    if (!demand) {
      demand = {
        attempted: false,
        input,
        leases: new Map(),
        retryPriority: null,
        sequence: nextSequence++,
      }
      demands.set(identity, demand)
    }
    demand.leases.set(lease, priority)
    reconcileQueue()

    let active = true
    return () => {
      if (!active) return
      active = false
      const current = demands.get(identity)
      if (!current) return
      current.leases.delete(lease)
      if (current.leases.size === 0) {
        demands.delete(identity)
        const task = tasks.get(identity)
        if (task?.state === 'queued') cancelQueued(task, true)
      } else {
        const task = tasks.get(identity)
        if (task?.state === 'queued') task.priority = effectivePriority(current)
      }
      reconcileQueue()
    }
  }

  function retry(input: RequestMermaidInput, priority: RequestMermaidPriority) {
    const identity = requestMermaidIdentity(input)
    const demand = demands.get(identity)
    if (!demand) return
    demand.attempted = false
    demand.retryPriority = priority
    resource.invalidate(identity)
    const task = tasks.get(identity)
    if (task?.state === 'queued') {
      cancelQueued(task, false)
    } else if (task?.state === 'started') {
      task.retryAfter = true
    }
    reconcileQueue()
  }

  function removeScope(accessScope: string) {
    const prefix = `${accessScope}\0`
    for (const identity of demands.keys()) {
      if (identity.startsWith(prefix)) demands.delete(identity)
    }
    for (const task of tasks.values()) {
      if (task.identity.startsWith(prefix) && task.state === 'queued') {
        tasks.delete(task.identity)
        task.deferred.reject(new Error('Mermaid render scope changed.'))
      }
    }
    resource.removeMatching((identity) => identity.startsWith(prefix))
    reconcileQueue()
  }

  function reset() {
    demands.clear()
    for (const task of tasks.values()) {
      if (task.state === 'queued') task.deferred.reject(new Error('Mermaid resource was reset.'))
    }
    for (const [identity, task] of tasks) {
      if (task.state === 'queued') tasks.delete(identity)
    }
    resource.clear()
    scopeTracker.reset()
  }

  return {
    resource,
    acquire,
    retry,
    activateScope: (accessScope: string) => scopeTracker.activate(accessScope),
    reset,
  }
}

const manager = createRequestMermaidResourceManager({ render: renderRequestMermaid })

export const requestMermaidResource = manager.resource
export const acquireRequestMermaid = manager.acquire
export const retryRequestMermaid = manager.retry
export const activateRequestMermaidScope = manager.activateScope
export const resetRequestMermaidResource = manager.reset

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve
    reject = nextReject
  })
  void promise.catch(() => {})
  return { promise, reject, resolve }
}

function yieldBrowserTask() {
  return new Promise<void>((resolve) => globalThis.setTimeout(resolve, 0))
}
