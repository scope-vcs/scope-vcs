import { parseVisibilityChange } from '@/api/history-inputs'
import { parseRouteFilePathSearch } from '@/lib/route-file'
import type { ProjectionPreviewAudience } from '@/api/types.generated'

export type UpdateSearch = {
  audience?: ProjectionPreviewAudience
  path?: string
  visibility_change?: string
}

export function parseUpdateSearch(search: Record<string, unknown>): UpdateSearch {
  return {
    audience: search.audience === 'private' || search.audience === 'public' ? search.audience : undefined,
    path: parseRouteFilePathSearch(search.path),
    visibility_change: parseVisibilityChange(search.visibility_change) ?? undefined,
  }
}

// The reader's broadest audience is the API default, so links omit it.
export function updateAudienceSearch(
  audience: ProjectionPreviewAudience,
  defaultAudience: ProjectionPreviewAudience,
): UpdateSearch {
  return audience === defaultAudience ? {} : { audience }
}
