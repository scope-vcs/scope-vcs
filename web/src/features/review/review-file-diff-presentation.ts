import type { ReviewDiffRenderBudget, ReviewFileDiff } from '../../api/types'
import { normalizeFilePath } from '../../components/file-system-tree-model'

export function reviewFileDiffEmptyLabel(diff: ReviewFileDiff | null) {
  const modeChange = reviewFileDiffModeChangeLabel(diff)
  if (modeChange) return modeChange
  if (diff?.kind === 'Renamed') return 'File renamed without changes'
  if (diff?.kind === 'Added') return 'Empty file added'
  if (diff?.kind === 'Deleted') return 'Empty file deleted'
  return 'No content changes'
}

export function reviewFileDiffModeChangeLabel(diff: ReviewFileDiff | null) {
  if (!diff?.old_mode || !diff.new_mode || diff.old_mode === diff.new_mode) {
    return null
  }
  return `Mode ${diff.old_mode} → ${diff.new_mode}`
}

export function reviewFileDiffRenameLabel(diff: ReviewFileDiff | null) {
  return diff?.old_path ? `Renamed from ${normalizeFilePath(diff.old_path)}` : null
}

export function reviewFileDiffOmittedLabel(budget: ReviewDiffRenderBudget) {
  return budget === 'standard'
    ? 'This diff is too large to show automatically'
    : 'This diff is too large to render'
}
