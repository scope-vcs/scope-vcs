import type {
  GitHubConnectionTestResponse,
  GitHubConnectionTestResultResponse,
} from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { CircleCheck, CircleX, LoaderCircle, PlugZap } from 'lucide-react'
import { useState } from 'react'
import { useSettingsAction } from './use-settings-action'

export function RepoGitHubConnectionTest({
  test,
}: {
  test: () => Promise<GitHubConnectionTestResponse>
}) {
  const { act, errorAlert, pending } = useSettingsAction<'test'>()
  const [results, setResults] = useState<GitHubConnectionTestResultResponse[] | null>(null)
  const run = () => act('test', 'The connection could not be tested', async () => {
    setResults((await test()).results)
  })
  return (
    <div className="space-y-2 border-t border-border pt-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="leading-5">Connection</div>
          <p className="leading-5 text-muted-foreground">
            Check that Scope can reach this repository with the access it needs.
          </p>
        </div>
        <Button
          disabled={pending !== null}
          onClick={() => void run()}
          size="sm"
          type="button"
          variant="secondary"
        >
          {pending ? <LoaderCircle className="size-3.5 animate-spin" /> : <PlugZap className="size-3.5" />}
          <span>Test connection</span>
        </Button>
      </div>
      {results && (
        <ul aria-label="Connection test results" aria-live="polite" className="space-y-1">
          {results.map((result) => (
            <li className="flex gap-2 leading-5" key={result.step}>
              {result.passed
                ? <CircleCheck aria-label="Passed" className="mt-0.5 size-4 shrink-0 text-success-strong" />
                : <CircleX aria-label="Failed" className="mt-0.5 size-4 shrink-0 text-danger-strong" />}
              <span className={result.passed ? 'text-muted-foreground' : 'text-foreground'}>{result.message}</span>
            </li>
          ))}
        </ul>
      )}
      {errorAlert}
    </div>
  )
}
