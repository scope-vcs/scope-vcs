import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { Plus, X } from 'lucide-react'
import { useState } from 'react'

export function RepoRequiredChecks({
  names,
  save,
}: {
  names: string[]
  save: (names: string[]) => Promise<unknown>
}) {
  const [draft, setDraft] = useState('')
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const name = draft.trim()
  const canAdd = name.length > 0 && !names.includes(name) && !pending

  async function update(next: string[], added: boolean) {
    setError(null)
    setPending(true)
    try {
      await save(next)
      if (added) setDraft('')
    } catch (cause) {
      setError(resourceErrorMessage(cause, 'Try again.'))
    } finally {
      setPending(false)
    }
  }

  return (
    <div className="space-y-2 border-t border-border pt-3">
      <div>
        <div className="leading-5">Required checks</div>
        <p className="leading-5 text-muted-foreground">
          A request can merge once each of these checks passes on GitHub. Use the names GitHub
          shows for its checks, such as ci / test.
        </p>
      </div>
      {names.length ? (
        <ul className="divide-y divide-border">
          {names.map((required) => (
            <li className="flex min-h-9 items-center justify-between gap-3" key={required}>
              <span className="min-w-0 truncate font-mono text-[13px]">{required}</span>
              <Button
                aria-label={`Stop requiring ${required}`}
                disabled={pending}
                onClick={() => void update(names.filter((existing) => existing !== required), false)}
                size="icon-sm"
                type="button"
                variant="ghost"
              >
                <X className="size-3.5" />
              </Button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="leading-5 text-muted-foreground">
          No checks are required. Workflows still run on maintainers’ pushes.
        </p>
      )}
      <form
        className="flex gap-2"
        onSubmit={(event) => {
          event.preventDefault()
          if (canAdd) void update([...names, name], true)
        }}
      >
        <Input
          aria-label="Check name"
          className="h-8"
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
      {error && (
        <PageErrorAlert className="mt-0" title="Required checks were not saved">
          {error}
        </PageErrorAlert>
      )}
    </div>
  )
}
