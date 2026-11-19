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
}: {
  isolatedRender: IsolatedFileRender
}) {
  let active = 0
  return async function highlightRepoFile(
    file: RepoFileContentResponse,
    signal?: AbortSignal,
  ): Promise<RepoFile> {
    signal?.throwIfAborted()
    const plain = { ...file, highlightedHtml: null }
    if (active >= REPO_FILE_HIGHLIGHT_LIMITS.maxConcurrentRenders) return plain
    const text = highlightableText(file)
    if (text === null) return plain

    active += 1
    try {
      const result = await isolatedRender({
        budget: { maxOutputBytes: REPO_FILE_HIGHLIGHT_LIMITS.maxOutputBytes },
        kind: 'file',
        path: file.path,
        text,
      }, REPO_FILE_HIGHLIGHT_LIMITS.deadlineMs, signal)
      signal?.throwIfAborted()
      if (result.kind === 'error') console.warn('[scope] file highlighting failed', file.path)
      return result.kind === 'html' ? { ...file, highlightedHtml: result.html } : plain
    } catch (error) {
      if (signal?.aborted) throw error
      if (!(error instanceof CodeRenderTransientError)) {
        console.warn('[scope] file highlighting failed', file.path, error)
      }
      return plain
    } finally {
      active -= 1
    }
  }
}

function highlightableText(file: RepoFileContentResponse) {
  if (file.content.kind !== 'text' || isRepositoryMarkdownPath(file.path)) return null
  if (file.content.text.length > REPO_FILE_HIGHLIGHT_LIMITS.maxInputBytes) return null
  const metrics = codeTextMetrics(file.content.text)
  return metrics.bytes > REPO_FILE_HIGHLIGHT_LIMITS.maxInputBytes ||
    metrics.lines > REPO_FILE_HIGHLIGHT_LIMITS.maxInputLines
    ? null
    : file.content.text
}
