import type { GitHubChecksResponse } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import type { CachedResource } from '@/lib/use-cached-resource'
import { RefreshCw } from 'lucide-react'
import { GITHUB_TRIGGER_SNIPPET, githubWorkflowsView } from './repo-github-checks-model'

export function RepoGitHubWorkflows({ checks }: { checks: CachedResource<GitHubChecksResponse> }) {
  const view = checks.value && githubWorkflowsView(checks.value)
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
          {view.attention.length > 0 && (
            <ul aria-label="Workflows that may not run on requests" className="divide-y divide-border">
              {view.attention.map((workflow) => (
                <li className="py-2" key={workflow.path}>
                  <div className="break-words font-mono text-[13px] leading-5 [overflow-wrap:anywhere]">
                    {workflow.path}
                  </div>
                  <p
                    className={workflow.runs.verdict === 'no'
                      ? 'leading-5 text-warning-strong'
                      : 'leading-5 text-muted-foreground'}
                  >
                    {workflow.runs.reason}
                  </p>
                </li>
              ))}
            </ul>
          )}
          {view.needsTrigger && (
            <pre className="overflow-x-auto rounded-md border border-border bg-muted/40 px-3 py-2 font-mono text-[13px] leading-5">
              <code>{GITHUB_TRIGGER_SNIPPET}</code>
            </pre>
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
