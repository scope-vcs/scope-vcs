import { createCachedResource } from '../../lib/cached-resource'
import type {
  HistoryEntrySummaryResponse,
  ProjectionPreviewAudience,
} from '../../api/types.generated'

export type RepositoryActivity = {
  audience: ProjectionPreviewAudience
  entry: HistoryEntrySummaryResponse | null
  head_oid: string | null
}

export const repositoryActivityResource = createCachedResource<RepositoryActivity>({
  maxEntries: 16,
  maxWeight: 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})
