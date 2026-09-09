import type { RepoRunDetail } from '@/api/types'
import { createCachedResource } from '../../lib/cached-resource'

type RunDetailSnapshot = {
  detail: RepoRunDetail
  routeSnapshot: string
  generation: number
}

export const runDetailResource = createCachedResource<RunDetailSnapshot>({ maxEntries: 16 })

export function initializeRunDetail(key: string, detail: RepoRunDetail) {
  const current = runDetailResource.read(key)
  const routeSnapshot = JSON.stringify(detail)
  if (current?.routeSnapshot !== routeSnapshot) {
    runDetailResource.write(key, { detail, routeSnapshot, generation: current?.generation ?? 0 }, runDetailResource.getSnapshot(key).version ?? '0')
  }
}

export async function refreshRunDetail(
  key: string,
  loadDetail: (signal?: AbortSignal) => Promise<RepoRunDetail>,
  forceAfterInFlight = false,
) {
  const current = runDetailResource.peek(key)
  if (!current) return
  let generation = 0
  const load = async (signal: AbortSignal) => {
    const detail = await loadDetail(AbortSignal.any([signal, AbortSignal.timeout(15_000)]))
    return { ...current, detail, generation }
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
