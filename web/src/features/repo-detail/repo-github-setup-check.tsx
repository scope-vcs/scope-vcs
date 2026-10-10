import type { GitHubConnectionResponse } from '@/api/types.generated'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { FlaskConical, LoaderCircle } from 'lucide-react'
import { useState } from 'react'
import { GITHUB_TRIGGER_SNIPPET, githubSetupCheckView } from './repo-github-setup-check-model'

export function RepoGitHubSetupCheck({
  github,
  startTest,
}: {
  github: GitHubConnectionResponse
  startTest: () => Promise<unknown>
}) {
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<{ title: string; message: string } | null>(null)
  const view = githubSetupCheckView(github.setup_check)

  async function testConnection() {
    setError(null)
    setPending(true)
    try {
      await startTest()
    } catch (cause) {
      setError({ title: 'The test could not start', message: resourceErrorMessage(cause, 'Try again.') })
    } finally {
      setPending(false)
    }
  }

  const testing = view?.running ?? false
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
          disabled={pending || testing}
          onClick={() => void testConnection()}
          size="sm"
          type="button"
          variant="secondary"
        >
          {pending || testing
            ? <LoaderCircle className="size-3.5 animate-spin" />
            : <FlaskConical className="size-3.5" />}
          <span>Test connection</span>
        </Button>
      </div>
      {view?.problem && (
        <p className="break-words leading-5 text-danger-strong" role="alert">{view.problem}</p>
      )}
      {error && <PageErrorAlert className="mt-0" title={error.title}>{error.message}</PageErrorAlert>}
    </div>
  )
}
