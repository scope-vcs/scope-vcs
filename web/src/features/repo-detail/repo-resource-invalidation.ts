import { invalidateRunHistoryScope } from '../runs/run-history-cache'
import { runDetailResource } from '../runs/run-detail-resource'
import { runLogCacheKey } from '../runs/run-log-cache'
import { runResourceNeedsRecovery } from '../runs/run-resource'
import { invalidateRequestQueues } from '../requests/request-queue-cache'
import { requestChangesResource } from '../requests/request-changes-resource'
import { refreshRequestState, requestStateResource } from '../requests/request-state-resource'
import { requestRatingsResource } from '../requests/request-ratings-resource'
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
import { githubWorkflowNamesResource } from '../runs/github-workflow-names-resource'
import { invalidateGitHubWorkflowRunDetails } from '../runs/github-workflow-run-detail-resource'

export function invalidateRepoSummaryResources(scope: string) {
  invalidateRequestQueues(scope)
}

export function invalidateRepoResources(scope: string, event?: RepoChangeEvent, summaryPending = false) {
  const changed = event && typeof event.kind === 'object' && 'RunChanged' in event.kind ? event.kind.RunChanged : null
  if (changed?.change === 'LogsAppended') return
  const repositoryChanged = event && typeof event.kind === 'object' && 'RepositoryChanged' in event.kind
  const recovery = !event || event.kind === 'Connected'
  const prefix = `${JSON.stringify([scope]).slice(0, -1)},`
  if (repositoryChanged || recovery || event?.kind === 'Lagged' || changed) {
    invalidateRunHistoryScope(scope, recovery)
  }
  if (repositoryChanged || recovery || event?.kind === 'Lagged' || changed?.change === 'StatusChanged') {
    runDetailResource.invalidateMatching((key) => key.startsWith(prefix) &&
      (!changed || key === runLogCacheKey(scope, changed.run_id)) &&
      (!recovery || runResourceNeedsRecovery(runDetailResource, key)))
  }
  if (!event || event.kind === 'Connected' || event.kind === 'Lagged' || typeof event.kind === 'object' && 'RepositoryChanged' in event.kind) {
    if (!summaryPending) invalidateRequestQueues(scope)
    repoSettingsResource.invalidate(scope)
    requestChangesResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    requestDiscussionReferenceResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    repoContentResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    repoFileResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    historyFeedResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    historyEntryResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    requestAttachmentResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    repositoryActivityResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    repositoryDependencyResource.invalidate(scope)
    runWorkflowsResource.invalidate(scope)
    requestActivityResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    refreshRequestState(scope)
    invalidateGitHubWorkflowRuns(scope)
    githubWorkflowNamesResource.invalidate(scope)
    invalidateGitHubWorkflowRunDetails(scope)
    requestRatingsResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
  } else if (typeof event.kind === 'object' && 'RequestStateChanged' in event.kind) {
    refreshRequestState(scope, event.kind.RequestStateChanged.request_id)
    if (!summaryPending) invalidateRequestQueues(scope)
  } else if (event.kind === 'DependenciesChanged') {
    repositoryDependencyResource.invalidate(scope)
  } else if (event.kind === 'GitHubWorkflowRunsChanged') {
    refreshRequestState(scope)
    invalidateGitHubWorkflowRuns(scope)
    githubWorkflowNamesResource.invalidate(scope)
    invalidateGitHubWorkflowRunDetails(scope)
  } else if (typeof event.kind === 'object' && 'GitHubWorkflowRunChanged' in event.kind) {
    invalidateGitHubWorkflowRunDetails(scope, event.kind.GitHubWorkflowRunChanged.github_run_id)
  } else if (typeof event.kind === 'object' && 'RequestTimelineChanged' in event.kind) {
    const timeline = event.kind.RequestTimelineChanged
    invalidateRequestQueues(scope)
    requestChangesResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0${timeline.request_id}\0`))
    requestDiscussionReferenceResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0${timeline.request_id}\0`))
    requestActivityResource.invalidate(requestActivityIdentity(scope, timeline.request_id))
    refreshRequestState(scope, timeline.request_id)
    requestRatingsResource.invalidate(`${scope}\0${timeline.request_id}`)
    requestAttachmentResource.invalidate(requestAttachmentResourceIdentity(scope, timeline.request_id))
  } else if (typeof event.kind === 'object' && 'RunChanged' in event.kind) {
    if (event.kind.RunChanged.change === 'StatusChanged') {
      invalidateRequestQueues(scope)
      requestActivityResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
    }
    const runId = event.kind.RunChanged.run_id
    requestStateResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`) &&
      (requestStateResource.getSnapshot(identity).pending ||
        Boolean(requestStateResource.peek(identity)?.state?.checks.checks.some((check) =>
          check.provider === 'native' && check.run_id === runId))))
  } else if (typeof event.kind === 'object' && 'RequestAttachmentChanged' in event.kind) {
    requestAttachmentResource.invalidate(requestAttachmentResourceIdentity(scope, event.kind.RequestAttachmentChanged.request_id))
  }
}
