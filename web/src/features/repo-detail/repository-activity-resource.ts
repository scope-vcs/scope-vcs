import type { HistoryEntrySummary, ProjectionPreviewAudience } from '../../api/types'
import { createCachedResource } from '../../lib/cached-resource'

export type RepositoryActivity = {
  audience: ProjectionPreviewAudience
  entry: HistoryEntrySummary | null
  head_oid: string | null
}

export const repositoryActivityResource = createCachedResource<RepositoryActivity>({
  maxEntries: 16,
  maxWeight: 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})
