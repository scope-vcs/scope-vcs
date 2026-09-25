import type { HistoryEntryDetailResponse } from '@/api/types.generated'

// Only the URL opens a diff; an update page starts on its file list.
export function historyFileSelection(
  search: { path?: string; visibility_change?: string },
  detail: Pick<HistoryEntryDetailResponse, 'files' | 'visibility_changes'> | null,
) {
  const visibilityId = search.visibility_change ?? null
  if (visibilityId) {
    const change = detail?.visibility_changes.find((change) => change.id === visibilityId)
    const path = search.path ?? change?.path ?? null
    return { path, file: change?.path === path ? change.file : null, visibilityId }
  }
  const path = search.path ?? null
  return { path, file: detail?.files.find((file) => file.path === path) ?? null, visibilityId }
}
