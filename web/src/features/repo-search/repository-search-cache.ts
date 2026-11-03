import type { ViewId } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import type { RepositorySearchResults } from './repository-search-model'

export const repositorySearchResource = createCachedResource<RepositorySearchResults>({
  maxEntries: 16,
  maxWeight: 2 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function repositorySearchIdentity(scope: string, view: ViewId, query: string) {
  return [scope, view, query].join('\0')
}
