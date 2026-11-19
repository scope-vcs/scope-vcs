import { Worker } from 'node:worker_threads'
import {
  createRepoFileHighlighter,
  REPO_FILE_HIGHLIGHT_LIMITS,
} from '../repo-detail/repo-file-highlight'
import {
  createReviewFileDiffRenderer,
  REVIEW_FILE_DIFF_MAX_CONCURRENT_RENDERS,
} from '../review/review-file-diff-renderer'
import { CODE_RENDER_STARTUP_DEADLINE_MS } from './code-render-contract'
import { createCodeRenderWorkerPool } from './code-render-worker-pool'

const renderersKey = Symbol.for('scope.code-renderers-v1')
const host = globalThis as unknown as Record<
  symbol,
  ReturnType<typeof createCodeRenderers> | undefined
>

function createCodeRenderers() {
  const isolatedRender = createCodeRenderWorkerPool(() => new Worker(
    import.meta.env.PROD
      ? new URL('../_workers/code-render-worker.mjs', import.meta.url)
      : new URL('./code-render-worker.ts', import.meta.url),
  ),
  REVIEW_FILE_DIFF_MAX_CONCURRENT_RENDERS + REPO_FILE_HIGHLIGHT_LIMITS.maxConcurrentRenders,
  CODE_RENDER_STARTUP_DEADLINE_MS)
  return {
    highlightRepoFile: createRepoFileHighlighter({ isolatedRender }),
    renderReviewFileDiff: createReviewFileDiffRenderer({ isolatedRender }),
  }
}

export const { highlightRepoFile, renderReviewFileDiff } = host[renderersKey] ??= createCodeRenderers()
