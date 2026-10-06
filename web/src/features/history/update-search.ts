import { parseVisibilityChange } from '@/api/history-inputs'
import { parseRouteFilePathSearch } from '@/lib/route-file'

export type UpdateSearch = {
  path?: string
  visibility_change?: string
}

export function parseUpdateSearch(search: Record<string, unknown>): UpdateSearch {
  return {
    path: parseRouteFilePathSearch(search.path),
    visibility_change: parseVisibilityChange(search.visibility_change) ?? undefined,
  }
}
