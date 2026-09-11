import type { ReviewFileDiff } from '@/api/types'
import type { CachedResource } from '@/lib/use-cached-resource'

export type CommitFileDiffState =
  | { diff: null; error: null; status: 'idle' }
  | { diff: null; error: null; status: 'loading' }
  | { diff: ReviewFileDiff; error: null; status: 'loaded' }
  | { diff: null; error: string; status: 'failed' }

export function resourceToDiffState(
  resource: CachedResource<ReviewFileDiff>,
): CommitFileDiffState {
  switch (resource.status) {
    case 'idle':
      return { diff: null, error: null, status: 'idle' }
    case 'loading':
      return { diff: null, error: null, status: 'loading' }
    case 'loaded':
      return { diff: resource.value, error: null, status: 'loaded' }
    case 'failed':
      return { diff: null, error: resource.error, status: 'failed' }
  }
}
