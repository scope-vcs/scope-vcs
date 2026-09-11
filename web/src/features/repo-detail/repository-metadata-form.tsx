import type { RepoSummary, UpdateRepoMetadataInput } from '@/api/types'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useState, type FormEvent } from 'react'
import { reconcileMetadataDraft, repositoryMetadata, sameMetadata, type MetadataDraft } from './repository-metadata-draft'

type Metadata = Pick<UpdateRepoMetadataInput, 'description' | 'website_url'>
type SaveState =
  | { status: 'idle' | 'saving' | 'saved' }
  | { status: 'failed'; message: string }

export function RepositoryMetadataForm({
  repo,
  save,
}: {
  repo: RepoSummary
  save: (metadata: Metadata) => Promise<RepoSummary>
}) {
  const [state, setState] = useState<SaveState>({ status: 'idle' })
  const incoming = repositoryMetadata(repo)
  const [draft, setDraft] = useState<MetadataDraft>(() => ({ source: incoming, value: incoming, conflict: false }))
  if (!sameMetadata(draft.source, incoming)) setDraft(reconcileMetadataDraft(draft, incoming))

  function edit(field: keyof Metadata, value: string) {
    setDraft((current) => ({ ...current, value: { ...current.value, [field]: value } }))
    setState({ status: 'idle' })
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (state.status === 'saving') return
    setState({ status: 'saving' })
    try {
      const saved = await save(draft.value)
      setDraft((current) => ({ ...current, value: repositoryMetadata(saved), conflict: false }))
      setState({ status: 'saved' })
    } catch (error) {
      setState({
        message: error instanceof Error ? error.message : 'Repository details could not be saved.',
        status: 'failed',
      })
    }
  }

  return (
    <SectionRows className="mt-0 border-b border-border">
      <SectionRow
        description="Help visitors understand the project. These details are public."
        title="Repository details"
      >
        <form
          onSubmit={submit}
        >
          <fieldset className="max-w-xl space-y-4" disabled={state.status === 'saving'}>
            <div>
              <label className="mb-1.5 block text-sm font-medium" htmlFor="repo-description">
                Description
              </label>
              <Input
                value={draft.value.description}
                onChange={(event) => edit('description', event.target.value)}
                id="repo-description"
                maxLength={160}
                name="description"
                placeholder="A short description of this project"
              />
            </div>
            <div>
              <label className="mb-1.5 block text-sm font-medium" htmlFor="repo-website">
                Website or documentation
              </label>
              <Input
                value={draft.value.website_url}
                onChange={(event) => edit('website_url', event.target.value)}
                id="repo-website"
                maxLength={2048}
                name="website_url"
                placeholder="https://example.com"
                type="url"
              />
            </div>
            {draft.conflict && (
              <p className="text-sm text-muted-foreground" role="status">
                Repository details changed while you were editing. Your draft is preserved.
                {' '}<button className="underline" type="button" onClick={() => {
                  setDraft((current) => ({ ...current, value: current.source, conflict: false }))
                  setState({ status: 'idle' })
                }}>Use updated details</button>
              </p>
            )}
            <div className="flex flex-wrap items-center gap-3">
              <Button size="sm" type="submit">
                {state.status === 'saving' ? 'Saving…' : 'Save details'}
              </Button>
              {state.status === 'saved' && (
                <output className="text-sm text-muted-foreground">Details saved.</output>
              )}
            </div>
            {state.status === 'failed' && (
              <p className="text-sm text-destructive" role="alert">{state.message}</p>
            )}
          </fieldset>
        </form>
      </SectionRow>
    </SectionRows>
  )
}
