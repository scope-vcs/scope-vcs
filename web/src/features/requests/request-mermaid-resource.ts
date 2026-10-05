import { createCachedResource } from '../../lib/cached-resource'
import { onViewerChange } from '../../lib/viewer-state'
import {
  renderRequestMermaid,
  type MermaidRenderInput,
  type MermaidRenderResult,
} from './request-mermaid-renderer'
import { createRequestAttachmentScopeTracker } from './request-attachment-scope-tracker'

export type RequestMermaidInput = MermaidRenderInput & { accessScope: string }
export type RequestMermaidResult = MermaidRenderResult
export type RequestMermaidPriority = 0 | 1

type Demand = {
  input: RequestMermaidInput
  leases: Map<symbol, RequestMermaidPriority>
  attempted: boolean
  retryPriority: RequestMermaidPriority | null
}

const RENDER_VERSION = '1'
const MAX_CACHE_ENTRIES = 64
const MAX_CACHE_WEIGHT = 4 * 1024 * 1024

export function requestMermaidIdentity(input: RequestMermaidInput) {
  return `${input.accessScope}\0${input.theme}\0${RENDER_VERSION}\0${input.source}`
}

export function createRequestMermaidResourceManager({
  render,
  yieldToBrowser = yieldBrowserTask,
  maxEntries = MAX_CACHE_ENTRIES,
  maxWeight = MAX_CACHE_WEIGHT,
}: {
  render: (input: MermaidRenderInput) => Promise<MermaidRenderResult>
  yieldToBrowser?: () => Promise<void>
  maxEntries?: number
  maxWeight?: number
}) {
  const retainedWeights = new WeakMap<RequestMermaidResult, number>()
  const resource = createCachedResource<RequestMermaidResult>({
    maxEntries,
    maxWeight,
    weightOf: (value) => retainedWeights.get(value) ?? value.svg.length * 2,
  })
  const demands = new Map<string, Demand>()
  let running = false
  const scopeTracker = createRequestAttachmentScopeTracker({
    maxOwners: 16,
    removePreviousScope: removeScope,
  })

  function nextDemand() {
    let nearby: [string, Demand] | undefined
    for (const entry of demands) {
      const [identity, demand] = entry
      const snapshot = resource.getSnapshot(identity)
      if (demand.attempted || snapshot.pending || snapshot.error !== null
        || (!snapshot.stale && snapshot.version === RENDER_VERSION)) continue
      if (demand.retryPriority === 0 || [...demand.leases.values()].includes(0)) return entry
      nearby ??= entry
    }
    return nearby
  }

  function pump() {
    if (running) return
    const next = nextDemand()
    if (!next) return
    const [identity, demand] = next
    running = true
    demand.attempted = true
    demand.retryPriority = null
    void run(identity, demand.input)
  }

  async function run(identity: string, input: RequestMermaidInput) {
    try {
      await resource.ensure(identity, RENDER_VERSION, async (signal) => {
        signal.throwIfAborted()
        const result = await render({ source: input.source, theme: input.theme })
        retainedWeights.set(result, (input.source.length + result.svg.length) * 2)
        return result
      })
    } finally {
      await yieldToBrowser().catch(() => {})
      running = false
      pump()
    }
  }

  function acquire(input: RequestMermaidInput, priority: RequestMermaidPriority) {
    const identity = requestMermaidIdentity(input)
    resource.read(identity)
    const demand: Demand = demands.get(identity) ?? {
      input,
      leases: new Map(),
      attempted: false,
      retryPriority: null,
    }
    const lease = Symbol()
    demand.leases.set(lease, priority)
    demands.set(identity, demand)
    pump()

    return () => {
      if (demands.get(identity) !== demand || !demand.leases.delete(lease)) return
      if (demand.leases.size === 0) demands.delete(identity)
      pump()
    }
  }

  function retry(input: RequestMermaidInput, priority: RequestMermaidPriority) {
    const identity = requestMermaidIdentity(input)
    const demand = demands.get(identity)
    if (!demand) return
    demand.attempted = false
    demand.retryPriority = priority
    resource.invalidate(identity)
    pump()
  }

  function removeScope(accessScope: string) {
    const prefix = `${accessScope}\0`
    for (const identity of demands.keys()) {
      if (identity.startsWith(prefix)) demands.delete(identity)
    }
    resource.removeMatching((identity) => identity.startsWith(prefix))
    pump()
  }

  function resetDemands() {
    demands.clear()
    scopeTracker.reset()
  }
  onViewerChange(resetDemands)

  function reset() {
    resetDemands()
    resource.clear()
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

function yieldBrowserTask() {
  return new Promise<void>((resolve) => globalThis.setTimeout(resolve, 0))
}
