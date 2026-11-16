import type {
  GitHubCatalogCheckResponse,
  GitHubCheckRunsVerdictResponse,
  GitHubChecksResponse,
  GitHubConnectionResponse,
  GitHubWorkflowSummaryResponse,
} from '../../api/types.generated'

export const GITHUB_TRIGGER_SNIPPET = `on:
  push:
    branches: ['scope/**']`

export type GitHubWorkflowGroup = {
  verdict: GitHubCheckRunsVerdictResponse
  reason: string | null
  workflows: GitHubWorkflowSummaryResponse[]
}

type GitHubWorkflowsView = {
  summary: string
  groups: GitHubWorkflowGroup[]
  runtimeNamed: string[]
}

const VERDICT_ORDER: Record<GitHubCheckRunsVerdictResponse, number> = { no: 0, maybe: 1, yes: 2 }

export function githubWorkflowsView(checks: GitHubChecksResponse): GitHubWorkflowsView {
  const { workflows } = checks
  const groups: GitHubWorkflowGroup[] = []
  for (const workflow of [...workflows].sort((a, b) => VERDICT_ORDER[a.runs.verdict] - VERDICT_ORDER[b.runs.verdict])) {
    const { verdict, reason } = workflow.runs
    const group = groups.find((candidate) => candidate.verdict === verdict && candidate.reason === reason)
    if (group) group.workflows.push(workflow)
    else groups.push({ verdict, reason, workflows: [workflow] })
  }
  const runtimeNamed = workflows
    .filter((workflow) => workflow.runtime_named_jobs.length > 0)
    .map((workflow) => `${workflow.name} (${workflow.runtime_named_jobs.join(', ')})`)
  return { groups, runtimeNamed, summary: workflowsSummary(workflows) }
}

function workflowsSummary(workflows: GitHubWorkflowSummaryResponse[]) {
  if (workflows.length === 0) return 'No workflows on main. Add one with this trigger so requests get checks.'
  const running = workflows.filter((workflow) => workflow.runs.verdict === 'yes').length
  const total = `${workflows.length} ${workflows.length === 1 ? 'workflow' : 'workflows'}`
  if (running < workflows.length) return `${running} of ${total} on main run on Scope requests.`
  return workflows.length === 1
    ? 'The workflow on main runs on Scope requests.'
    : `All ${total} on main run on Scope requests.`
}

export function searchWorkflows(groups: GitHubWorkflowGroup[], query: string) {
  const needle = query.trim().toLowerCase()
  return groups
    .flatMap((group) => group.workflows)
    .filter((workflow) => [workflow.path, workflow.name, ...workflow.triggers]
      .some((text) => text.toLowerCase().includes(needle)))
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
