import { repoCollaborationResource } from './repo-collaboration-resource'
import { requestChangesResource, requestDiscussionReferenceResource } from '../requests/request-changes-resource'
import { repoContentResource } from './repo-content-cache'
import { repoFileResource } from './repo-file-cache'
import type { RepoChangeEvent } from '../../api/types.generated'
import { requestActivityIdentity, requestActivityResource } from '../requests/request-activity-resource'
import { requestAttachmentResource, requestAttachmentResourceIdentity } from '../requests/request-attachment-resource'
import { repositoryActivityResource } from './repository-activity-resource'

export function invalidateRepoResources(scope: string, event?: RepoChangeEvent) {
  if (!event || event.kind === 'Lagged' || typeof event.kind === 'object' && 'RepositoryChanged' in event.kind) {
    repoCollaborationResource.invalidate(scope)
    requestChangesResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    requestDiscussionReferenceResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    repoContentResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    repoFileResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    requestAttachmentResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    repositoryActivityResource.invalidate(scope)
    requestActivityResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
  } else if (typeof event.kind === 'object' && 'RequestTimelineChanged' in event.kind) {
    const requestIdentity = `${scope}\0${event.kind.RequestTimelineChanged.request_id}`
    requestChangesResource.invalidateMatching((identity) => identity.startsWith(`${requestIdentity}\0`))
    requestDiscussionReferenceResource.invalidateMatching((identity) => identity.startsWith(`${requestIdentity}\0`))
    requestActivityResource.invalidate(requestActivityIdentity(scope, event.kind.RequestTimelineChanged.request_id))
    requestAttachmentResource.invalidate(requestAttachmentResourceIdentity(scope, event.kind.RequestTimelineChanged.request_id))
  } else if (typeof event.kind === 'object' && 'RequestAttachmentChanged' in event.kind) {
    requestAttachmentResource.invalidate(requestAttachmentResourceIdentity(scope, event.kind.RequestAttachmentChanged.request_id))
  }
}
