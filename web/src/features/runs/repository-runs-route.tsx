import type { RepoGitHubWorkflowRunsInput, RepoParams } from '@/api/types'
import { PageErrorAlert } from '@/components/page-error-alert'
import { WorkbenchBar, WorkbenchBody, WorkbenchPane } from '@/components/page-header'
import { useCachedResource } from '@/lib/use-cached-resource'
import { GitHubWorkflowRunsPage } from '@/features/runs/github-workflow-runs'
import {
  loadRepoGitHubWorkflowRun,
  loadRepoGitHubWorkflowRuns,
  loadRepoGitHubWorkflowNames,
  loadRepoRunPage,
} from '@/routes/-run-history-actions'
import { loadRepoSettingsData, startRepoGitHubAuthorization } from '@/routes/-repo-settings-actions'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback, useMemo } from 'react'
import { useRepoLayout } from '../repo-detail/repo-layout-context'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { RunsCiEmptyState } from './runs-ci-empty-state'
import { RunsPageError } from './runs-page-error'
import { RunsPagePending } from './runs-page-pending'
import { type RunsPageHandoff, runsPageResource } from './runs-page-resource'

export function RepositoryRunsRoute({ initialPage, params }: { initialPage: RunsPageHandoff | null; params: RepoParams }) {
  const { userId, isLoaded } = useAuth()
  const { repo } = useRepoLayout()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const { owner, repo: repoName } = params
  const page = useCachedResource({
    fallbackError: 'Runs are unavailable.',
    identity: scope,
    initialValue: scope && initialPage?.scope === scope ? initialPage.page : null,
    load: useCallback(
      (signal: AbortSignal) => loadRepoRunPage({ data: { owner, repo: repoName }, signal }),
      [owner, repoName],
    ),
    resource: runsPageResource,
  })
  const loadRuns = useCallback(
    (data: RepoGitHubWorkflowRunsInput, signal: AbortSignal) => loadRepoGitHubWorkflowRuns({ data, signal }),
    [],
  )
  const loadNames = useCallback(
    (data: RepoParams, signal: AbortSignal) => loadRepoGitHubWorkflowNames({ data, signal }),
    [],
  )
  const loadRunDetail = useCallback(
    (runId: string, signal: AbortSignal) =>
      loadRepoGitHubWorkflowRun({ data: { owner, repo: repoName, run_id: runId }, signal }),
    [owner, repoName],
  )
  const value = page.value
  const github = useMemo(() => ({
    configured: value?.kind === 'not-connected' && value.configured,
    loadSettings: loadRepoSettingsData,
    startAuthorization: startRepoGitHubAuthorization,
  }), [value])

  if (!value) return page.error ? <RunsPageError error={page.error} /> : <RunsPagePending />

  if (value.kind === 'github') {
    return (
      <GitHubWorkflowRunsPage
        initialNames={value.names}
        initialRuns={value.github}
        key={`${owner}/${repoName}`}
        loadNames={loadNames}
        loadRunDetail={loadRunDetail}
        loadRuns={loadRuns}
        params={params}
      />
    )
  }

  return (
    <WorkbenchPane>
      <WorkbenchBar title="Runs" />
      <WorkbenchBody>
        {value.kind === 'denied' ? (
          <PageErrorAlert title="Runs unavailable">
            Sign in as the owner or a repository member to view runs.
          </PageErrorAlert>
        ) : (
          <div className="pt-7">
            <RunsCiEmptyState github={github} params={params} />
          </div>
        )}
      </WorkbenchBody>
    </WorkbenchPane>
  )
}
