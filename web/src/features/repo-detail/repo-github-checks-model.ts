import type {
  GitHubCatalogCheckResponse,
  GitHubChecksResponse,
  GitHubConnectionResponse,
  GitHubWorkflowSummaryResponse,
} from '../../api/types.generated'

export const GITHUB_TRIGGER_SNIPPET = `on:
  push:
    branches: ['scope/**']`

type GitHubWorkflowsView = {
  summary: string
  attention: GitHubWorkflowSummaryResponse[]
  needsTrigger: boolean
  runtimeNamed: string[]
}

export function githubWorkflowsView(checks: GitHubChecksResponse): GitHubWorkflowsView {
  const { workflows } = checks
  const attention = workflows.filter((workflow) => workflow.runs.verdict !== 'yes')
  const runtimeNamed = workflows
    .filter((workflow) => workflow.runtime_named_jobs.length > 0)
    .map((workflow) => `${workflow.name} (${workflow.runtime_named_jobs.join(', ')})`)
  const view = {
    attention,
    needsTrigger: workflows.length === 0 || attention.some((workflow) => workflow.runs.verdict === 'no'),
    runtimeNamed,
  }
  if (workflows.length === 0) {
    return { ...view, summary: 'No workflows on main. Add one with this trigger so requests get checks.' }
  }
  const running = workflows.length - attention.length
  const total = `${workflows.length} ${workflows.length === 1 ? 'workflow' : 'workflows'}`
  if (attention.length === 0) {
    return {
      ...view,
      summary: workflows.length === 1
        ? 'The workflow on main runs on Scope requests.'
        : `All ${total} on main run on Scope requests.`,
    }
  }
  return {
    ...view,
    summary: `${running} of ${total} on main run on Scope requests.`,
  }
}

type GitHubCheckStatus = { text: string; tone: 'muted' | 'warning' }

export function githubCheckStatus(check: GitHubCatalogCheckResponse | undefined): GitHubCheckStatus {
  if (!check) return { text: 'Not found in workflows or on GitHub', tone: 'warning' }
  const required = check.required_on_github ? ', required on GitHub' : ''
  switch (check.runs?.verdict) {
    case 'yes':
      return { text: `Runs on Scope requests${required}`, tone: 'muted' }
    case 'maybe':
      return { text: `${(check.runs.reason ?? 'May not run on every request').replace(/\.$/, '')}${required}`, tone: 'muted' }
    case 'no':
      return { text: `Does not run on Scope requests${required}`, tone: 'warning' }
    case undefined:
      if (!check.reported_on_main) {
        return { text: 'Required on GitHub but not reported as a check run, so Scope cannot wait for it', tone: 'warning' }
      }
      return check.required_on_github
        ? { text: 'Required on GitHub, reported by another app', tone: 'muted' }
        : { text: 'Reported on main by another app', tone: 'muted' }
  }
}

export function githubChecksVersion(contentVersion: number, github: GitHubConnectionResponse | null) {
  const connection = github?.connection
  if (!connection || connection.disconnected) return null
  return [
    contentVersion,
    connection.github_full_name,
    connection.connected_at_unix,
    connection.open_review_bot_pull_requests,
  ].join(':')
}
