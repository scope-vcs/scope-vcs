import type { GitHubConnectionResponse } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { FlaskConical, LoaderCircle } from 'lucide-react'
import { GITHUB_TRIGGER_SNIPPET, githubSetupCheckView } from './repo-github-setup-check-model'
import { useSettingsAction } from './use-settings-action'

export function RepoGitHubSetupCheck({
  github,
  startTest,
}: {
  github: GitHubConnectionResponse
  startTest: () => Promise<unknown>
}) {
  const { act, errorAlert, pending } = useSettingsAction<'test'>()
  const view = githubSetupCheckView(github.setup_check)
  const busy = pending !== null || (view?.running ?? false)
  return (
    <div className="space-y-2 border-t border-border pt-3">
      <div>
        <div className="leading-5">Run workflows on requests</div>
        <p className="leading-5 text-muted-foreground">
          Add this trigger to each workflow that should run on Scope requests. Keep the
          triggers it already has.
        </p>
      </div>
      <pre className="overflow-x-auto rounded-md border border-border bg-muted/40 px-3 py-2 font-mono text-[13px] leading-5">
        <code>{GITHUB_TRIGGER_SNIPPET}</code>
      </pre>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p aria-live="polite" className="min-w-0 leading-5 text-muted-foreground">
          {view?.status ?? 'Test the connection to check that workflows start on Scope’s branches.'}
        </p>
        <Button
          disabled={busy}
          onClick={() => void act('test', 'The test could not start', startTest)}
          size="sm"
          type="button"
          variant="secondary"
        >
          {busy
            ? <LoaderCircle className="size-3.5 animate-spin" />
            : <FlaskConical className="size-3.5" />}
          <span>Test connection</span>
        </Button>
      </div>
      {view?.problem && (
        <p className="break-words leading-5 text-danger-strong" role="alert">{view.problem}</p>
      )}
      {errorAlert}
    </div>
  )
}
