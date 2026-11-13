import type { GitHubWorkflowNamesResponse, GitHubWorkflowRunListResponse } from '@/api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'

export type RunsPage =
  | { kind: 'github'; github: GitHubWorkflowRunListResponse; names: GitHubWorkflowNamesResponse }
  | { kind: 'not-connected'; configured: boolean }
  | { kind: 'denied' }

export type RunsPageHandoff = { scope: string; page: RunsPage }

export const runsPageResource = createCachedResource<RunsPage>({
  maxEntries: 12,
  maxWeight: 2 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})
