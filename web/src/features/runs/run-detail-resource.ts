import { createCachedResource } from '../../lib/cached-resource'
import { runResourceNeedsRecovery } from './run-resource'
import type { RepositoryRunDetailResponse } from '@/api/types.generated'

export type RunDetailSnapshot = {
  updatedAt?: number
  detail: RepositoryRunDetailResponse
  generation: number
}

export const runDetailResource = createCachedResource<RunDetailSnapshot>({ maxEntries: 16 })

export function initializeRunDetail(key: string, detail: RepositoryRunDetailResponse) {
  runDetailResource.seed(key, { detail, generation: 0, updatedAt: Date.now() }, '0')
}

export async function refreshRunDetail(
  key: string,
  loadDetail: (signal?: AbortSignal) => Promise<RepositoryRunDetailResponse>,
  forceAfterInFlight = false,
  recovery = false,
) {
  if (recovery && !runResourceNeedsRecovery(runDetailResource, key)) return
  const current = runDetailResource.peek(key)
  if (!current) return
  let generation = 0
  const load = async (signal: AbortSignal) => {
    const detail = await loadDetail(AbortSignal.any([signal, AbortSignal.timeout(15_000)]))
    return { ...current, detail, generation, updatedAt: Date.now() }
  }
  if (runDetailResource.getSnapshot(key).pending) {
    const value = await runDetailResource.ensure(key, runDetailResource.getSnapshot(key).version!, load)
    if (!forceAfterInFlight) {
      if (!value) throw runDetailResource.getSnapshot(key).error
      return
    }
  }
  generation = Number(runDetailResource.getSnapshot(key).version ?? 0) + 1
  runDetailResource.invalidate(key)
  await runDetailResource.load(key, String(generation), load)
}

export async function loadRunDetailSnapshot(key: string, loadDetail: (signal?: AbortSignal) => Promise<RepositoryRunDetailResponse>, signal: AbortSignal): Promise<RunDetailSnapshot> {
  const detail = await loadDetail(AbortSignal.any([signal, AbortSignal.timeout(15_000)]))
  return { detail, generation: Number(runDetailResource.getSnapshot(key).version ?? 0), updatedAt: Date.now() }
}
