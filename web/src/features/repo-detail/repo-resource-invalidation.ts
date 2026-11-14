import { invalidateRequestQueues } from '../requests/request-queue-cache'
import { requestChangesResource } from '../requests/request-changes-resource'
import { refreshRequestState } from '../requests/request-state-resource'
import { requestRatingsResource } from '../requests/request-ratings-resource'
import { refreshRequestDiscussionReferences, requestDiscussionReferenceResource } from '../requests/request-changes-discussion-references'
import type { RepoChangeEvent } from '../../api/types.generated'
import { requestActivityIdentity, requestActivityResource } from '../requests/request-activity-resource'
import { requestAttachmentResource, requestAttachmentResourceIdentity } from '../requests/request-attachment-resource'
import { repositoryActivityResource } from './repository-activity-resource'
import { repositoryDependencyResource } from './repository-dependency-resource'
import { repoSettingsResource } from './repo-settings-resource'
import { repoGitHubChecksResource } from './repo-github-checks-resource'
import { repoContentResource } from './repo-content-cache'
import { repoFileResource } from './repo-file-cache'
import { repositoryVersionKey } from './repository-version'
import { historyEntryResource, historyFeedResource } from '../history/history-resource-cache'
import { invalidateGitHubWorkflowRuns } from '../runs/github-workflow-runs-resource'
import { runsPageResource } from '../runs/runs-page-resource'
import { githubWorkflowNamesResource } from '../runs/github-workflow-names-resource'
import { invalidateGitHubWorkflowRunDetails } from '../runs/github-workflow-run-detail-resource'

export function invalidateRepoSummaryResources(scope: string) {
  invalidateRequestQueues(scope)
}

type RepoResourceInvalidation = {
  summaryPending?: boolean
  versioned?: boolean
}

export function invalidateRepoResources(
  scope: string,
  event?: RepoChangeEvent,
  { summaryPending = false, versioned = false }: RepoResourceInvalidation = {},
) {
  const repositoryChanged = event && typeof event.kind === 'object' && 'RepositoryChanged' in event.kind
  const inScope = (identity: string) => identity.startsWith(`${scope}\0`)
  if (!event || event.kind === 'Connected' || event.kind === 'Lagged' || repositoryChanged) {
    if (!summaryPending) invalidateRequestQueues(scope)
    repoSettingsResource.invalidate(scope)
    repoGitHubChecksResource.invalidate(scope)
    requestChangesResource.invalidateMatching(inScope)
    requestDiscussionReferenceResource.invalidateMatching(inScope)
    historyFeedResource.invalidateMatching(inScope)
    historyEntryResource.invalidateMatching(inScope)
    requestAttachmentResource.invalidateMatching(inScope)
    if (repositoryChanged || !versioned) {
      repoContentResource.invalidateMatching(inScope)
      repoFileResource.invalidateMatching(inScope)
      repositoryActivityResource.invalidateMatching(inScope)
      repositoryDependencyResource.invalidate(scope)
    }
    runsPageResource.invalidate(scope)
    requestActivityResource.invalidateMatching(inScope)
    refreshRequestState(scope)
    invalidateGitHubWorkflowRuns(scope)
    githubWorkflowNamesResource.invalidate(scope)
    invalidateGitHubWorkflowRunDetails(scope)
    requestRatingsResource.invalidateMatching(inScope)
  } else if (typeof event.kind === 'object' && 'RequestStateChanged' in event.kind) {
    const requestId = event.kind.RequestStateChanged.request_id
    const version = `\0${repositoryVersionKey({ kind: 'request', id: requestId })}`
    repoContentResource.invalidateMatching((identity) => inScope(identity) && identity.endsWith(version))
    repoFileResource.invalidateMatching((identity) => inScope(identity) && identity.includes(`${version}\0`))
    refreshRequestState(scope, requestId)
    refreshRequestDiscussionReferences(scope, requestId)
    if (!summaryPending) invalidateRequestQueues(scope)
  } else if (event.kind === 'DependenciesChanged') {
    repositoryDependencyResource.invalidate(scope)
  } else if (event.kind === 'GitHubWorkflowRunsChanged') {
    refreshRequestState(scope)
    if (!summaryPending) invalidateRequestQueues(scope)
    invalidateGitHubWorkflowRuns(scope)
    githubWorkflowNamesResource.invalidate(scope)
    repoGitHubChecksResource.invalidate(scope)
    invalidateGitHubWorkflowRunDetails(scope)
  } else if (typeof event.kind === 'object' && 'GitHubWorkflowRunChanged' in event.kind) {
    invalidateGitHubWorkflowRunDetails(scope, event.kind.GitHubWorkflowRunChanged.github_run_id)
  } else if (typeof event.kind === 'object' && 'RequestTimelineChanged' in event.kind) {
    const timeline = event.kind.RequestTimelineChanged
    invalidateRequestQueues(scope)
    requestChangesResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0${timeline.request_id}\0`))
    refreshRequestDiscussionReferences(scope, timeline.request_id)
    requestActivityResource.invalidate(requestActivityIdentity(scope, timeline.request_id))
    refreshRequestState(scope, timeline.request_id)
    requestRatingsResource.invalidate(`${scope}\0${timeline.request_id}`)
    requestAttachmentResource.invalidate(requestAttachmentResourceIdentity(scope, timeline.request_id))
  } else if (typeof event.kind === 'object' && 'RequestAttachmentChanged' in event.kind) {
    requestAttachmentResource.invalidate(requestAttachmentResourceIdentity(scope, event.kind.RequestAttachmentChanged.request_id))
  }
}
