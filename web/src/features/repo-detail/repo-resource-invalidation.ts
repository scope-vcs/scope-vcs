import { requestQueueResource } from '../requests/request-queue-cache'
import { requestChangesResource } from '../requests/request-changes-resource'
import { requestChecksResource } from '../requests/request-checks-resource'
import { requestAutoMergeIdentity, requestAutoMergeResource } from '../requests/request-auto-merge-resource'
import { requestDiscussionReferenceResource } from '../requests/request-changes-discussion-references'
import type { RepoChangeEvent } from '../../api/types.generated'
import { requestActivityIdentity, requestActivityResource } from '../requests/request-activity-resource'
import { requestAttachmentResource, requestAttachmentResourceIdentity } from '../requests/request-attachment-resource'
import { repositoryActivityResource } from './repository-activity-resource'
import { repositoryDependencyResource } from './repository-dependency-resource'
import { repoCollaborationResource } from './repo-collaboration-resource'
import { repoContentResource } from './repo-content-cache'
import { repoFileResource } from './repo-file-cache'

export function invalidateRepoResources(scope: string, event?: RepoChangeEvent) {
  if (!event || event.kind === 'Lagged' || typeof event.kind === 'object' && 'RepositoryChanged' in event.kind) {
    requestQueueResource.invalidate(scope)
    repoCollaborationResource.invalidate(scope)
    requestChangesResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    requestDiscussionReferenceResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    repoContentResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    repoFileResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    requestAttachmentResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    repositoryActivityResource.invalidate(scope)
    repositoryDependencyResource.invalidate(scope)
    requestActivityResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    requestChecksResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    requestAutoMergeResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
  } else if (event.kind === 'DependenciesChanged') {
    repositoryDependencyResource.invalidate(scope)
  } else if (typeof event.kind === 'object' && 'RequestTimelineChanged' in event.kind) {
    const timeline = event.kind.RequestTimelineChanged
    requestQueueResource.invalidate(scope)
    requestChangesResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0${timeline.request_id}\0`))
    requestDiscussionReferenceResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0${timeline.request_id}\0`))
    requestActivityResource.invalidate(requestActivityIdentity(scope, timeline.request_id))
    requestAutoMergeResource.invalidate(requestAutoMergeIdentity(scope, timeline.request_id))
    requestAttachmentResource.invalidate(requestAttachmentResourceIdentity(scope, timeline.request_id))
  } else if (typeof event.kind === 'object' && 'RunChanged' in event.kind) {
    // A run status change can stop an active auto-merge intent and append
    // request activity. Without a request id, refresh those views in scope.
    if (event.kind.RunChanged.change === 'StatusChanged') {
      requestQueueResource.invalidate(scope)
      requestActivityResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    }
    requestChecksResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    requestAutoMergeResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
  } else if (typeof event.kind === 'object' && 'RequestAttachmentChanged' in event.kind) {
    requestAttachmentResource.invalidate(requestAttachmentResourceIdentity(scope, event.kind.RequestAttachmentChanged.request_id))
  }
}
