import type { CommitDetail, ReviewFileDiff } from '@/api/types'
import type {
  ReviewDiffFullRender,
  ReviewFileDiffResource,
} from '@/features/review/review-file-diff-resource'

export type CommitDetailState =
  | { commit: null; error: null; status: 'idle' }
  | { commit: CommitDetail; error: null; status: 'loaded' }
  | { commit: null; error: string; status: 'failed' }

export type CommitFileDiffState =
  | { diff: null; error: null; status: 'idle' }
  | { diff: null; error: null; status: 'loading' }
  | { diff: ReviewFileDiff; error: null; fullRender: ReviewDiffFullRender | null; status: 'loaded' }
  | { diff: null; error: string; status: 'failed' }

export function resourceToDiffState(
  resource: ReviewFileDiffResource,
): CommitFileDiffState {
  switch (resource.status) {
    case 'idle':
      return { diff: null, error: null, status: 'idle' }
    case 'loading':
      return { diff: null, error: null, status: 'loading' }
    case 'loaded':
      return { diff: resource.value, error: null, fullRender: resource.fullRender, status: 'loaded' }
    case 'failed':
      return { diff: null, error: resource.error, status: 'failed' }
  }
}
