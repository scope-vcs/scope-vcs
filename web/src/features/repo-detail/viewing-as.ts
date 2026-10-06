import { parseViewId, type RepoViews } from '../../api/repo-views'
import type { ViewId } from '../../api/types.generated'

export type ViewingAsSearch = { view?: ViewId }

export function parseViewingAsSearch(search: Record<string, unknown>): ViewingAsSearch {
  if (search.view === undefined) return {}
  try {
    return { view: parseViewId(search.view) }
  } catch {
    return {}
  }
}

export function resolveViewingAs(
  views: RepoViews,
  reader: ViewId,
  requested: ViewId | null | undefined,
): ViewId {
  return requested && views.mayRead(reader, requested) ? requested : reader
}

export function viewingAsSearch(view: ViewId, reader: ViewId): { view: ViewId | undefined } {
  return { view: view === reader ? undefined : view }
}
