import type { GitHubConnectionResponse } from '@/api/types.generated'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { Plus } from 'lucide-react'
import { useEffect, useState } from 'react'

function sameNames(left: string[], right: string[]) {
  return left.length === right.length && left.every((name) => right.includes(name))
}

export function RepoRequiredChecks({
  github,
  save,
}: {
  github: GitHubConnectionResponse
  save: (names: string[]) => Promise<GitHubConnectionResponse>
}) {
  const names = github.required_checks
  const discovered = github.setup_check?.check_names ?? []
  const [selection, setSelection] = useState({ source: names, names, conflict: false })
  const [draft, setDraft] = useState('')
  const [manual, setManual] = useState(false)
  const [pending, setPending] = useState(false)
  const [saved, setSaved] = useState<string[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const dirty = !sameNames(selection.names, selection.source)
  const candidates = [...new Set([...discovered, ...names, ...selection.names])]
  const name = draft.trim()
  const canAdd = name.length > 0 && !selection.names.includes(name) && !pending

  useEffect(() => {
    setSelection((current) => {
      if (sameNames(current.source, names)) return current
      const edited = !sameNames(current.source, current.names)
      return {
        source: names,
        names: edited ? current.names : names,
        conflict: edited && !sameNames(current.names, names),
      }
    })
  }, [names])

  function select(next: string[]) {
    setSelection((current) => ({ ...current, names: next }))
    setError(null)
    setSaved(null)
  }

  async function update() {
    setError(null)
    setPending(true)
    try {
      const result = await save(selection.names)
      setSelection({ source: result.required_checks, names: result.required_checks, conflict: false })
      setSaved(result.required_checks)
    } catch (cause) {
      setError(resourceErrorMessage(cause, 'Try again.'))
    } finally {
      setPending(false)
    }
  }

  return (
    <div className="space-y-2 border-t border-border pt-3">
      <div>
        <div className="leading-5 font-medium">Required before merge</div>
        <p className="leading-5 text-muted-foreground">
          Select the GitHub results a request must pass. Saved requirements apply to future pushes.
          Existing revisions keep their recorded requirements.
        </p>
      </div>
      {candidates.length ? (
        <ul aria-label="Results required before merge" className="divide-y divide-border">
          {candidates.map((candidate) => (
            <li key={candidate}>
              <label className="flex min-h-11 cursor-pointer flex-wrap items-center gap-x-3 gap-y-1 py-2">
                <input
                  checked={selection.names.includes(candidate)}
                  className="size-4 shrink-0 accent-primary"
                  disabled={pending}
                  onChange={(event) => select(event.target.checked
                    ? [...selection.names, candidate]
                    : selection.names.filter((existing) => existing !== candidate))}
                  type="checkbox"
                />
                <span className="min-w-0 flex-1 break-words font-mono text-[13px] [overflow-wrap:anywhere]">{candidate}</span>
                <span className="text-xs text-muted-foreground">
                  {discovered.includes(candidate) ? 'Observed in connection test' : 'Not observed in latest test'}
                </span>
              </label>
            </li>
          ))}
        </ul>
      ) : (
        <p className="leading-5 text-muted-foreground">
          No results selected. Test the connection to discover results, or enter an exact name.
          Workflows need the scope/** push trigger to run on Scope requests.
        </p>
      )}
      {selection.conflict && (
        <p className="leading-5 text-muted-foreground" role="status">
          Requirements changed while you were editing. Your selection is preserved.
          {' '}<button className="underline" disabled={pending} onClick={() => {
            setSelection({ source: names, names, conflict: false })
            setSaved(null)
            setError(null)
          }} type="button">Use saved requirements</button>
        </p>
      )}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p aria-live="polite" className="leading-5 text-muted-foreground">
          {saved && sameNames(saved, names) && !dirty ? 'Requirements saved for future pushes.' : `${selection.names.length} ${selection.names.length === 1 ? 'result' : 'results'} selected${dirty ? ' · unsaved changes' : ''}`}
        </p>
        <Button disabled={!dirty || pending} onClick={() => void update()} size="sm" type="button">
          {pending ? 'Saving…' : 'Save requirements'}
        </Button>
      </div>
      <Button aria-expanded={manual} disabled={pending} onClick={() => setManual(!manual)} size="sm" type="button" variant="ghost">
        Enter an exact result name…
      </Button>
      {manual && (
        <form
          className="flex gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            if (!canAdd) return
            select([...selection.names, name])
            setDraft('')
          }}
        >
          <Input
            aria-label="Exact result name"
            className="h-8 min-w-0"
            disabled={pending}
            onChange={(event) => setDraft(event.target.value)}
            placeholder="ci / test"
            value={draft}
          />
          <Button disabled={!canAdd} size="sm" type="submit" variant="secondary">
            <Plus className="size-3.5" />
            <span>Add</span>
          </Button>
        </form>
      )}
      {error && (
        <PageErrorAlert className="mt-0" title="Requirements were not saved">
          {error}
        </PageErrorAlert>
      )}
    </div>
  )
}
