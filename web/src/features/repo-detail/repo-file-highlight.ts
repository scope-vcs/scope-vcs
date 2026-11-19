import type { RepoFileContentResponse } from '../../api/types.generated'
import { isRepositoryMarkdownPath } from '../../components/repository-markdown'
import {
  codeTextMetrics,
  type CodeRenderFileInput,
  type CodeRenderWorkerResult,
} from '../code-render/code-render-contract'
import { CodeRenderTransientError } from '../code-render/code-render-worker-pool'

export type RepoFile = RepoFileContentResponse & { highlightedHtml: string | null }

export const REPO_FILE_HIGHLIGHT_LIMITS = Object.freeze({
  deadlineMs: 1_500,
  maxConcurrentRenders: 2,
  maxInputBytes: 100 * 1024,
  maxInputLines: 2_000,
  maxOutputBytes: 2 * 1024 * 1024,
})

type IsolatedFileRender = (
  input: CodeRenderFileInput,
  deadlineMs: number,
  signal?: AbortSignal,
) => Promise<CodeRenderWorkerResult>

export function createRepoFileHighlighter({
  isolatedRender,
  state = { active: 0 },
}: {
  isolatedRender: IsolatedFileRender
  state?: { active: number }
}) {
  return async function highlightRepoFile(
    file: RepoFileContentResponse,
    signal?: AbortSignal,
  ): Promise<RepoFile> {
    const text = highlightableText(file)
    if (text === null || state.active >= REPO_FILE_HIGHLIGHT_LIMITS.maxConcurrentRenders) {
      return { ...file, highlightedHtml: null }
    }

    state.active += 1
    try {
      const result = await isolatedRender({
        budget: { maxOutputBytes: REPO_FILE_HIGHLIGHT_LIMITS.maxOutputBytes },
        kind: 'file',
        path: file.path,
        text,
      }, REPO_FILE_HIGHLIGHT_LIMITS.deadlineMs, signal)
      if (result.kind === 'error') console.warn('[scope] file highlighting failed', file.path)
      return { ...file, highlightedHtml: result.kind === 'html' ? result.html : null }
    } catch (error) {
      if (signal?.aborted) throw error
      if (!(error instanceof CodeRenderTransientError)) {
        console.warn('[scope] file highlighting failed', file.path, error)
      }
      return { ...file, highlightedHtml: null }
    } finally {
      state.active -= 1
    }
  }
}

function highlightableText(file: RepoFileContentResponse) {
  if (file.content.kind !== 'text' || isRepositoryMarkdownPath(file.path)) return null
  const metrics = codeTextMetrics(file.content.text)
  return metrics.bytes > REPO_FILE_HIGHLIGHT_LIMITS.maxInputBytes ||
    metrics.lines > REPO_FILE_HIGHLIGHT_LIMITS.maxInputLines
    ? null
    : file.content.text
}
