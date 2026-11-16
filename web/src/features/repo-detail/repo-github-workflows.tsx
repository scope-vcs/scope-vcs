import type { GitHubChecksResponse, GitHubWorkflowSummaryResponse } from '@/api/types.generated'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { NavigationSearch } from '@/components/navigation-search'
import { Button } from '@/components/ui/button'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import type { CachedResource } from '@/lib/use-cached-resource'
import { CircleCheck, ChevronRight, CircleHelp, ExternalLink, RefreshCw, TriangleAlert } from 'lucide-react'
import { useState } from 'react'
import {
  GITHUB_TRIGGER_SNIPPET,
  githubWorkflowsView,
  searchWorkflows,
  workflowFileUrl,
  type GitHubWorkflowGroup,
} from './repo-github-checks-model'

const VERDICT_DISPLAY = {
  no: { title: 'Not run on requests', Icon: TriangleAlert, tone: 'text-warning' },
  maybe: { title: 'May not run on every request', Icon: CircleHelp, tone: 'text-muted-foreground' },
  yes: { title: 'Run on requests', Icon: CircleCheck, tone: 'text-success' },
} as const

export function RepoGitHubWorkflows({
  checks,
  repositoryUrl,
}: {
  checks: CachedResource<GitHubChecksResponse>
  repositoryUrl: string
}) {
  const [query, setQuery] = useState('')
  const view = checks.value && githubWorkflowsView(checks.value)
  const matches = view && query.trim() ? searchWorkflows(view.groups, query) : null
  return (
    <div className="space-y-2 border-t border-border pt-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="leading-5">Run workflows on requests</div>
        {view && (
          <Button disabled={checks.refreshing} onClick={checks.retry} size="sm" type="button" variant="secondary">
            <RefreshCw className={checks.refreshing ? 'size-3.5 animate-spin' : 'size-3.5'} />
            <span>Check GitHub again</span>
          </Button>
        )}
      </div>
      {checks.status === 'loading' && <TextSkeleton length="long" />}
      {checks.status === 'failed' && (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="min-w-0 flex-1 leading-5 text-danger-strong" role="alert">{checks.error}</p>
          <Button onClick={checks.retry} size="sm" type="button" variant="secondary">Try again</Button>
        </div>
      )}
      {view && (
        <>
          <p aria-live="polite" className="leading-5 text-muted-foreground">{view.summary}</p>
          {view.groups.length === 0 ? <CopyableCodeBlock copyLabel="Copy trigger" value={GITHUB_TRIGGER_SNIPPET} /> : (
            <>
              <NavigationSearch
                clearLabel="Clear workflow search"
                label="Find a workflow"
                onChange={setQuery}
                placeholder="Find a workflow by name or trigger"
                shortcut={false}
                value={query}
              />
              {matches && (
                <div>
                  <p aria-live="polite" className="leading-5 text-muted-foreground">
                    {matches.length === 0
                      ? `No workflow on main matches “${query.trim()}”.`
                      : `${matches.length} of ${checks.value.workflows.length} workflows`}
                  </p>
                  <ul aria-label="Matching workflows">
                    {matches.map((workflow) => (
                      <WorkflowRow key={workflow.path} repositoryUrl={repositoryUrl} showVerdict workflow={workflow} />
                    ))}
                  </ul>
                </div>
              )}
              <div className="divide-y divide-border border-y border-border" hidden={matches !== null}>
                {view.groups.map((group) => (
                  <WorkflowGroup group={group} key={`${group.verdict}:${group.reason}`} repositoryUrl={repositoryUrl} />
                ))}
              </div>
            </>
          )}
          {view.runtimeNamed.length > 0 && (
            <p className="leading-5 text-muted-foreground">
              Some jobs set their result names while they run, so those names appear once they report on main:
              {' '}{view.runtimeNamed.join('; ')}.
            </p>
          )}
          {checks.error && (
            <p className="leading-5 text-danger-strong" role="alert">
              {checks.error}{' '}
              <button className="underline" onClick={checks.retry} type="button">Try again</button>
            </p>
          )}
        </>
      )}
    </div>
  )
}

function WorkflowGroup({ group, repositoryUrl }: { group: GitHubWorkflowGroup; repositoryUrl: string }) {
  const [open, setOpen] = useState(false)
  const { title, Icon, tone } = VERDICT_DISPLAY[group.verdict]
  return (
    <div>
      <button
        aria-expanded={open}
        className="grid w-full grid-cols-[auto_auto_minmax(0,1fr)_auto] items-center gap-2 px-1 py-2 text-left hover:bg-muted/50 focus-visible:outline-2 focus-visible:outline-ring"
        onClick={() => setOpen(!open)}
        type="button"
      >
        <ChevronRight aria-hidden="true" className={`size-3.5 text-muted-foreground transition-transform motion-reduce:transition-none ${open ? 'rotate-90' : ''}`} />
        <Icon aria-hidden="true" className={`size-3.5 ${tone}`} />
        <span className="min-w-0">
          <span className="block font-medium leading-5">{title}</span>
          {group.reason && <span className="block leading-5 text-muted-foreground [overflow-wrap:anywhere]">{group.reason}</span>}
        </span>
        <span
          className={`min-w-6 rounded-full px-2 text-center text-xs leading-5 tabular-nums ${group.verdict === 'no' ? 'bg-warning-soft text-warning-strong' : 'bg-muted text-muted-foreground'}`}
        >
          {group.workflows.length}
        </span>
      </button>
      {open && (
        <div className="space-y-2 pb-3 pl-1 sm:pl-12">
          {group.verdict === 'no' && <CopyableCodeBlock copyLabel="Copy trigger" value={GITHUB_TRIGGER_SNIPPET} />}
          <ul aria-label={title}>
            {group.workflows.map((workflow) => (
              <WorkflowRow key={workflow.path} repositoryUrl={repositoryUrl} workflow={workflow} />
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}

function WorkflowRow({
  repositoryUrl,
  showVerdict = false,
  workflow,
}: {
  repositoryUrl: string
  showVerdict?: boolean
  workflow: GitHubWorkflowSummaryResponse
}) {
  const fileName = workflow.path.split('/').at(-1) ?? workflow.path
  const { title, Icon, tone } = VERDICT_DISPLAY[workflow.runs.verdict]
  return (
    <li className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 border-t border-border/70 py-1.5 first:border-t-0 sm:grid-cols-[minmax(0,1fr)_auto_auto]">
      <span className="col-start-1 row-start-1 flex min-w-0 items-center gap-2">
        {showVerdict && <Icon aria-label={title} className={`size-3.5 shrink-0 ${tone}`} role="img" />}
        <span className="font-mono text-[13px] leading-5 [overflow-wrap:anywhere]" title={workflow.path}>{fileName}</span>
        {workflow.name !== fileName && <span className="truncate leading-5 text-muted-foreground">{workflow.name}</span>}
      </span>
      <span className="col-span-full row-start-2 text-[13px] leading-5 text-muted-foreground sm:col-span-1 sm:col-start-2 sm:row-start-1">
        {workflow.triggers.join(', ')}
      </span>
      <a
        aria-label={`Open ${fileName} on GitHub`}
        className="col-start-2 row-start-1 rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring sm:col-start-3"
        href={workflowFileUrl(repositoryUrl, workflow.path)}
        rel="noreferrer"
        target="_blank"
      >
        <ExternalLink aria-hidden="true" className="size-3.5" />
      </a>
      {showVerdict && workflow.runs.reason && (
        <span className={`col-span-full text-[13px] leading-5 [overflow-wrap:anywhere] ${workflow.runs.verdict === 'no' ? 'text-warning-strong' : 'text-muted-foreground'}`}>
          {workflow.runs.reason}
        </span>
      )}
    </li>
  )
}
