import { requestQueueResource } from '../requests/request-queue-cache'
import type { RepoChangeEvent } from '../../api/types.generated'
import { requestActivityIdentity, requestActivityResource } from '../requests/request-activity-resource'
import { requestAttachmentResource, requestAttachmentResourceIdentity } from '../requests/request-attachment-resource'
import { repositoryActivityResource } from './repository-activity-resource'
import { repositoryDependencyResource } from './repository-dependency-resource'

export function invalidateRepoResources(scope: string, event?: RepoChangeEvent) {
  if (!event || event.kind === 'Lagged' || typeof event.kind === 'object' && 'RepositoryChanged' in event.kind) {
    requestQueueResource.invalidate(scope)
    requestAttachmentResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    repositoryActivityResource.invalidate(scope)
    repositoryDependencyResource.invalidate(scope)
    requestActivityResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
  } else if (event.kind === 'DependenciesChanged') {
    repositoryDependencyResource.invalidate(scope)
  } else if (typeof event.kind === 'object' && 'RequestTimelineChanged' in event.kind) {
    requestQueueResource.invalidate(scope)
    requestActivityResource.invalidate(requestActivityIdentity(scope, event.kind.RequestTimelineChanged.request_id))
    requestAttachmentResource.invalidate(requestAttachmentResourceIdentity(scope, event.kind.RequestTimelineChanged.request_id))
  } else if (typeof event.kind === 'object' && 'RequestAttachmentChanged' in event.kind) {
    requestAttachmentResource.invalidate(requestAttachmentResourceIdentity(scope, event.kind.RequestAttachmentChanged.request_id))
  }
}
