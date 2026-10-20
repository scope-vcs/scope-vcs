import type {
  RequestComparisonMode,
  RequestDiscussionAnchorInput,
  RequestDiscussionLineRange,
} from '@/api/types.generated'
import { displayRouteFilePath } from '../../lib/route-file'
import type { ReviewDiffLine, ReviewDiffRenderedLines } from '../review/review-file-diff-lines'
import type { RequestAttachmentDraftTarget } from './request-attachment-drafts'
import type { RequestDiscussion } from './request-discussion-types'

export type LineDiscussionComparison = {
  commitOid: string | null
  mode: RequestComparisonMode
  path: string
  revisionId: string
}

export type PlacedLineDiscussion = {
  discussion: RequestDiscussion
  lines: RequestDiscussionLineRange
}

export type FileLineDiscussion = {
  discussion: RequestDiscussion
  lines: RequestDiscussionLineRange | null
}

type LineDiscussionPosition =
  | { kind: 'lines'; lines: RequestDiscussionLineRange }
  | { kind: 'outdated' }

export function canStartLineDiscussion(mode: RequestComparisonMode) {
  return mode !== 'push'
}

export function lineDiscussionsForFile(
  discussions: RequestDiscussion[],
  comparison: LineDiscussionComparison,
  rendered: ReviewDiffRenderedLines | null,
) {
  const placed: PlacedLineDiscussion[] = []
  const fileLevel: FileLineDiscussion[] = []
  const ordered = [...discussions].sort((left, right) => left.opened_position - right.opened_position)
  for (const discussion of ordered) {
    const position = lineDiscussionPosition(discussion, comparison)
    if (!position) continue
    if (position.kind === 'outdated') {
      fileLevel.push({ discussion, lines: null })
    } else if (rendered?.[position.lines.side].has(position.lines.end_line)) {
      placed.push({ discussion, lines: position.lines })
    } else {
      fileLevel.push({ discussion, lines: position.lines })
    }
  }
  return { fileLevel, placed }
}

function lineDiscussionPosition(
  discussion: RequestDiscussion,
  comparison: LineDiscussionComparison,
): LineDiscussionPosition | null {
  const anchor = discussion.anchor
  if (!anchor?.lines || !anchor.path || !canStartLineDiscussion(comparison.mode)) return null
  if (displayRouteFilePath(anchor.path) !== displayRouteFilePath(comparison.path)) return null
  if (anchor.revision_id === comparison.revisionId) {
    const sameDiff = comparison.mode === 'commit'
      ? anchor.commit_oid === comparison.commitOid
      : anchor.commit_oid === null
    return sameDiff ? { kind: 'lines', lines: anchor.lines } : null
  }
  const latest = anchor.latest
  if (comparison.mode !== 'request' || latest?.revision_id !== comparison.revisionId) return null
  return latest.kind === 'mapped'
    ? {
        kind: 'lines',
        lines: { side: anchor.lines.side, start_line: latest.start_line, end_line: latest.end_line },
      }
    : { kind: 'outdated' }
}

export function extendLineSelection(
  current: RequestDiscussionLineRange | null,
  line: ReviewDiffLine,
  extend: boolean,
): RequestDiscussionLineRange {
  if (!extend || !current || current.side !== line.side) {
    return { side: line.side, start_line: line.line, end_line: line.line }
  }
  return {
    side: line.side,
    start_line: Math.min(current.start_line, line.line),
    end_line: Math.max(current.end_line, line.line),
  }
}

export function lineDiscussionAnchor(
  comparison: LineDiscussionComparison,
  lines: RequestDiscussionLineRange,
): RequestDiscussionAnchorInput {
  return {
    commit_oid: comparison.mode === 'commit' ? comparison.commitOid : null,
    lines,
    path: comparison.path,
    revision_id: comparison.revisionId,
  }
}

export function lineDiscussionDraftTarget(anchor: RequestDiscussionAnchorInput): RequestAttachmentDraftTarget {
  return `line-discussion:${JSON.stringify([
    anchor.revision_id,
    anchor.commit_oid,
    anchor.path,
    anchor.lines?.side,
    anchor.lines?.start_line,
    anchor.lines?.end_line,
  ])}`
}

export function lineRangeLabel(lines: RequestDiscussionLineRange) {
  const range = lines.start_line === lines.end_line
    ? `line ${lines.start_line}`
    : `lines ${lines.start_line}-${lines.end_line}`
  return `${range} (${lines.side})`
}
