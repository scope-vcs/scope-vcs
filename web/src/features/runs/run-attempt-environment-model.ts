import { formatBytes } from '../../lib/format-bytes'
import type {
  RepositoryRunAttemptResponse,
  RepositoryRunCacheResponse,
} from '@/api/types.generated'

/** Worth a marker on the collapsed Environment control: a cache started
 * cold or its facts never arrived. */
export function cachesNeedAttention(caches: readonly RepositoryRunCacheResponse[]) {
  return caches.some((cache) =>
    !cache.observation || cache.observation.preparation.kind === 'cold')
}

export function cacheStateLabel(cache: RepositoryRunCacheResponse) {
  const preparation = cache.observation?.preparation
  if (!preparation) return 'not reported'
  return preparation.kind
}

export function cacheStateClass(cache: RepositoryRunCacheResponse) {
  switch (cacheStateLabel(cache)) {
    case 'exact':
    case 'compatible':
      return 'text-success'
    case 'cold':
      return 'text-warning'
    default:
      return 'text-muted-foreground'
  }
}

/** Why a cold cache was cold, for its hover text. */
export function cacheStateDetail(cache: RepositoryRunCacheResponse) {
  const preparation = cache.observation?.preparation
  if (!preparation) return 'Cache facts were not reported for this attempt.'
  return preparation.kind === 'cold' ? coldReasonLabel(preparation.reason) : null
}

export function cacheSizeLabel(cache: RepositoryRunCacheResponse) {
  const observation = cache.observation
  return observation ? formatBytes(observation.size_bytes) : null
}

export function cacheTimingLabel(cache: RepositoryRunCacheResponse) {
  const observation = cache.observation
  return observation ? formatMilliseconds(observation.prepare_ms) : null
}

export function cacheSetupLabel(
  cacheSetup: RepositoryRunAttemptResponse['cache_setup'],
) {
  return cacheSetup ? `Set up in ${formatMilliseconds(cacheSetup.wall_ms)}` : null
}

export function pinnedImageLabel(image: string | null) {
  if (!image) return 'Image not pinned yet'
  const digest = image.includes('@sha256:')
    ? image.split('@sha256:').pop() ?? image
    : image
  return `sha256:${digest.slice(0, 12)}`
}

function coldReasonLabel(reason: string) {
  switch (reason) {
    case 'metadata-missing':
      return 'No reusable entry for this identity'
    case 'metadata-invalid':
      return 'Cache metadata was invalid'
    case 'metadata-not-ready':
      return 'Cached volume was not ready'
    default:
      return 'Cache was cold'
  }
}

function formatMilliseconds(milliseconds: number) {
  if (milliseconds < 1_000) return `${milliseconds}ms`
  return `${(milliseconds / 1_000).toFixed(1)}s`
}
