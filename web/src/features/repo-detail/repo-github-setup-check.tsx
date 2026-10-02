import type { GitHubConnectionResponse } from '@/api/types.generated'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { Check, FlaskConical, LoaderCircle, Plus } from 'lucide-react'
import { useState } from 'react'
import { GITHUB_TRIGGER_SNIPPET, githubSetupCheckView } from './repo-github-setup-check-model'

/**
 * How a maintainer makes workflows run on Scope requests: the trigger to add,
 * and a test that pushes main and lists the checks GitHub ran, each of which
 * can be required with one click. The test's result comes from the settings
 * data, so it outlasts this page.
 */
export function RepoGitHubSetupCheck({
  github,
  requireCheck,
  startTest,
}: {
  github: GitHubConnectionResponse
  requireCheck: (name: string) => Promise<unknown>
  startTest: () => Promise<unknown>
}) {
  const [pending, setPending] = useState<string | null>(null)
  const [error, setError] = useState<{ title: string; message: string } | null>(null)
  const view = githubSetupCheckView(github.setup_check, github.required_checks)

  async function act(key: string, title: string, action: () => Promise<unknown>) {
    setError(null)
    setPending(key)
    try {
      await action()
    } catch (cause) {
      setError({ title, message: resourceErrorMessage(cause, 'Try again.') })
    } finally {
      setPending(null)
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
          disabled={pending !== null || testing}
          onClick={() => void act('test', 'The test could not start', startTest)}
          size="sm"
          type="button"
          variant="secondary"
        >
          {pending === 'test' || testing
            ? <LoaderCircle className="size-3.5 animate-spin" />
            : <FlaskConical className="size-3.5" />}
          <span>Test connection</span>
        </Button>
      </div>
      {view?.problem && (
        <p className="break-words leading-5 text-danger-strong" role="alert">{view.problem}</p>
      )}
      {view?.candidates.length ? (
        <ul aria-label="Checks GitHub ran" className="divide-y divide-border">
          {view.candidates.map((candidate) => (
            <li className="flex min-h-9 items-center justify-between gap-3" key={candidate.name}>
              <span className="min-w-0 truncate font-mono text-[13px]">{candidate.name}</span>
              {candidate.required ? (
                <span className="flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
                  <Check className="size-3.5" />
                  Required
                </span>
              ) : (
                <Button
                  aria-label={`Require ${candidate.name}`}
                  disabled={pending !== null}
                  onClick={() => void act(
                    candidate.name,
                    'The check was not required',
                    () => requireCheck(candidate.name),
                  )}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  {pending === candidate.name
                    ? <LoaderCircle className="size-3.5 animate-spin" />
                    : <Plus className="size-3.5" />}
                  <span>Require</span>
                </Button>
              )}
            </li>
          ))}
        </ul>
      ) : null}
      {error && <PageErrorAlert className="mt-0" title={error.title}>{error.message}</PageErrorAlert>}
    </div>
  )
}
