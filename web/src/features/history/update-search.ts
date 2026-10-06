import { parseHistoryView, parseVisibilityChange } from '@/api/history-inputs'
import { parseRouteFilePathSearch } from '@/lib/route-file'
import type { ViewId } from '@/api/types.generated'

export type UpdateSearch = {
  view?: ViewId
  path?: string
  visibility_change?: string
}

export function parseUpdateSearch(search: Record<string, unknown>): UpdateSearch {
  return {
    view: search.view ? parseHistoryView(search.view) : undefined,
    path: parseRouteFilePathSearch(search.path),
    visibility_change: parseVisibilityChange(search.visibility_change) ?? undefined,
  }
}

export function updateViewSearch(
  view: ViewId,
  defaultView: ViewId,
): UpdateSearch {
  return view === defaultView ? {} : { view }
}
