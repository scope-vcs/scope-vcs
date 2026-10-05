import type { GitHubConnectionResponse } from '@/api/types.generated'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { cn } from '@/lib/utils'
import { Download, LoaderCircle } from 'lucide-react'
import { useState } from 'react'
import {
  RUN_IMPORT_COUNT_HINT,
  githubRunImportView,
  parseRunImportCount,
} from './repo-github-run-import-model'

export function RepoGitHubRunImport({
  connected,
  github,
  importNow,
  saveCount,
}: {
  connected: boolean
  github: GitHubConnectionResponse
  importNow: () => Promise<unknown>
  saveCount: (count: number) => Promise<unknown>
}) {
  const [draft, setDraft] = useState(String(github.run_import_count))
  const [pending, setPending] = useState<'save' | 'import' | null>(null)
  const [error, setError] = useState<{ title: string; message: string } | null>(null)
  const count = parseRunImportCount(draft)
  const view = githubRunImportView(github.run_import)
  const canSave = count !== null && count !== github.run_import_count && pending === null
  const canImport = connected
    && github.run_import_count > 0
    && !(view?.inProgress && !view.retrying)
    && pending === null

  async function act(key: 'save' | 'import', title: string, action: () => Promise<unknown>) {
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

  return (
    <div className="space-y-2 border-t border-border pt-3">
      <div>
        <div className="leading-5">Import recent runs</div>
        <p className="leading-5 text-muted-foreground">
          When this repository connects, Scope reads this many of GitHub’s most recent workflow
          runs for the Runs page. 0 imports none.
        </p>
      </div>
      <form
        className="flex flex-wrap items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault()
          if (canSave && count !== null) void act('save', 'The count was not saved', () => saveCount(count))
        }}
      >
        <Input
          aria-invalid={count === null}
          aria-label="Recent runs to import"
          className="h-8 w-24"
          disabled={pending !== null}
          inputMode="numeric"
          max={1000}
          min={0}
          onChange={(event) => setDraft(event.target.value)}
          type="number"
          value={draft}
        />
        <Button disabled={!canSave} size="sm" type="submit" variant="secondary">
          {pending === 'save' && <LoaderCircle className="size-3.5 animate-spin" />}
          <span>Save</span>
        </Button>
        {connected && (
          <Button
            disabled={!canImport}
            onClick={() => void act('import', 'The import could not start', importNow)}
            size="sm"
            type="button"
            variant="ghost"
          >
            {pending === 'import' || (view?.inProgress && !view.retrying)
              ? <LoaderCircle className="size-3.5 animate-spin" />
              : <Download className="size-3.5" />}
            <span>Import now</span>
          </Button>
        )}
      </form>
      {count === null && <p className="leading-5 text-danger-strong">{RUN_IMPORT_COUNT_HINT}</p>}
      {view && (
        <p
          aria-live="polite"
          className={cn('break-words leading-5', view.failed ? 'text-danger-strong' : 'text-muted-foreground')}
        >
          {view.status}
        </p>
      )}
      {error && <PageErrorAlert className="mt-0" title={error.title}>{error.message}</PageErrorAlert>}
    </div>
  )
}
