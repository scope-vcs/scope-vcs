import type { RepoChangeEvent } from '../../api/types.generated'
import { requestActivityIdentity, requestActivityResource } from '../requests/request-activity-resource'
import { repositoryActivityResource } from './repository-activity-resource'

export function invalidateRepoActivityResources(scope: string, event?: RepoChangeEvent) {
  if (!event || event.kind === 'Lagged' || typeof event.kind === 'object' && 'RepositoryChanged' in event.kind) {
    repositoryActivityResource.invalidate(scope)
    requestActivityResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
  } else if (typeof event.kind === 'object' && 'RequestTimelineChanged' in event.kind) {
    requestActivityResource.invalidate(requestActivityIdentity(scope, event.kind.RequestTimelineChanged.request_id))
  }
}
