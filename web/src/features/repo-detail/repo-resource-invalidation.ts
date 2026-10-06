import { invalidateRunHistoryScope } from '../runs/run-history-cache'
import { runDetailResource } from '../runs/run-detail-resource'
import { runLogCacheKey } from '../runs/run-log-cache'
import { runResourceNeedsRecovery } from '../runs/run-resource'
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
import { repoSettingsResource } from './repo-settings-resource'
import { repoContentResource } from './repo-content-cache'
import { repoFileResource } from './repo-file-cache'
import { historyEntryResource, historyFeedResource } from '../history/history-resource-cache'
import { runWorkflowsResource } from '../runs/run-workflows-resource'
import { invalidateGitHubWorkflowRuns } from '../runs/github-workflow-runs-resource'
import { invalidateGitHubWorkflowRunDetails } from '../runs/github-workflow-run-detail-resource'

export function invalidateRepoSummaryResources(scope: string) {
  requestQueueResource.invalidate(scope)
}

export function invalidateRepoResources(scope: string, event?: RepoChangeEvent, summaryPending = false) {
  const changed = event && typeof event.kind === 'object' && 'RunChanged' in event.kind ? event.kind.RunChanged : null
  const repositoryChanged = event && typeof event.kind === 'object' && 'RepositoryChanged' in event.kind
  const recovery = !event || event.kind === 'Connected'
  const prefix = `${JSON.stringify([scope]).slice(0, -1)},`
  if (repositoryChanged || recovery || event?.kind === 'Lagged' || changed && changed.change !== 'LogsAppended') {
    invalidateRunHistoryScope(scope, recovery)
  }
  if (repositoryChanged || recovery || event?.kind === 'Lagged' || changed?.change === 'StatusChanged') {
    runDetailResource.invalidateMatching((key) => key.startsWith(prefix) &&
      (!changed || key === runLogCacheKey(scope, changed.run_id)) &&
      (!recovery || runResourceNeedsRecovery(runDetailResource, key)))
  }
  if (!event || event.kind === 'Connected' || event.kind === 'Lagged' || typeof event.kind === 'object' && 'RepositoryChanged' in event.kind) {
    if (!summaryPending) requestQueueResource.invalidate(scope)
    repoSettingsResource.invalidate(scope)
    requestChangesResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    requestDiscussionReferenceResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    repoContentResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    repoFileResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    historyFeedResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    historyEntryResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    requestAttachmentResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    repositoryActivityResource.invalidate(scope)
    repositoryDependencyResource.invalidate(scope)
    runWorkflowsResource.invalidate(scope)
    requestActivityResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    requestChecksResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    requestAutoMergeResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    invalidateGitHubWorkflowRuns(scope)
    invalidateGitHubWorkflowRunDetails(scope)
  } else if (event.kind === 'DependenciesChanged') {
    repositoryDependencyResource.invalidate(scope)
  } else if (event.kind === 'GitHubWorkflowRunsChanged') {
    invalidateGitHubWorkflowRuns(scope)
    invalidateGitHubWorkflowRunDetails(scope)
  } else if (typeof event.kind === 'object' && 'GitHubWorkflowRunChanged' in event.kind) {
    invalidateGitHubWorkflowRunDetails(scope, event.kind.GitHubWorkflowRunChanged.github_run_id)
  } else if (typeof event.kind === 'object' && 'RequestTimelineChanged' in event.kind) {
    const timeline = event.kind.RequestTimelineChanged
    requestQueueResource.invalidate(scope)
    requestChangesResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0${timeline.request_id}\0`))
    requestDiscussionReferenceResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0${timeline.request_id}\0`))
    requestActivityResource.invalidate(requestActivityIdentity(scope, timeline.request_id))
    requestAutoMergeResource.invalidate(requestAutoMergeIdentity(scope, timeline.request_id))
    requestAttachmentResource.invalidate(requestAttachmentResourceIdentity(scope, timeline.request_id))
  } else if (typeof event.kind === 'object' && 'RunChanged' in event.kind) {
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
