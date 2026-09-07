import type { HistoryEntryDetail } from '@/api/types'

export function historySelectedFilePath(
  path: string | undefined,
  files: readonly { path: string }[] | undefined,
  dismissed: boolean,
): string | null {
  if (dismissed) return null
  return path ?? files?.[0]?.path ?? null
}

export function historyFileSelection(
  search: { path?: string; visibility_change?: string },
  detail: Pick<HistoryEntryDetail, 'files' | 'visibility_changes'> | null,
  dismissed: boolean,
) {
  if (dismissed) return { path: null, file: null, visibilityId: null }
  const visibilityId = search.visibility_change ?? null
  if (visibilityId) {
    const change = detail?.visibility_changes.find((change) => change.id === visibilityId)
    const path = search.path ?? change?.path ?? null
    return { path, file: change?.path === path ? change.file : null, visibilityId }
  }
  const path = historySelectedFilePath(search.path, detail?.files, false)
  return { path, file: detail?.files.find((file) => file.path === path) ?? null, visibilityId }
}
