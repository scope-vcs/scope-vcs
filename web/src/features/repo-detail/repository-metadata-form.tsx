import type { RepoSummaryResponse } from '@/api/types.generated'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useEffect, useReducer, useState, type FormEvent } from 'react'
import {
  acceptMetadataSave,
  reconcileMetadataDraft,
  repositoryMetadata,
  type MetadataDraft,
  type RepositoryMetadata,
} from './repository-metadata-draft'

type Metadata = RepositoryMetadata
type SaveState =
  | { status: 'idle' | 'saving' | 'saved' }
  | { status: 'failed'; message: string }
type DraftAction =
  | { type: 'edit'; field: keyof Metadata; value: string }
  | { type: 'incoming'; value: Metadata }
  | { type: 'saved'; value: Metadata }
  | { type: 'use-incoming' }

export function RepositoryMetadataForm({
  repo,
  save,
}: {
  repo: RepoSummaryResponse
  save: (metadata: Metadata) => Promise<RepoSummaryResponse>
}) {
  const [state, setState] = useState<SaveState>({ status: 'idle' })
  const repoDescription = repo.description
  const repoWebsiteUrl = repo.website_url
  const [draft, dispatchDraft] = useReducer(
    metadataDraftReducer,
    {
      description: repoDescription,
      website_url: repoWebsiteUrl,
    },
    (value): MetadataDraft => {
      const initial = repositoryMetadata(value)
      return { source: initial, value: initial, conflict: false }
    },
  )
  useEffect(() => {
    dispatchDraft({
      type: 'incoming',
      value: repositoryMetadata({
        description: repoDescription,
        website_url: repoWebsiteUrl,
      }),
    })
  }, [repoDescription, repoWebsiteUrl])

  function edit(field: keyof Metadata, value: string) {
    dispatchDraft({ type: 'edit', field, value })
    setState({ status: 'idle' })
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (state.status === 'saving') return
    setState({ status: 'saving' })
    try {
      const saved = await save(draft.value)
      dispatchDraft({ type: 'saved', value: repositoryMetadata(saved) })
      setState({ status: 'saved' })
    } catch (error) {
      setState({
        message: resourceErrorMessage(error, 'Repository details could not be saved.'),
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
                  dispatchDraft({ type: 'use-incoming' })
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

function metadataDraftReducer(current: MetadataDraft, action: DraftAction): MetadataDraft {
  switch (action.type) {
    case 'edit':
      return { ...current, value: { ...current.value, [action.field]: action.value } }
    case 'incoming':
      return reconcileMetadataDraft(current, action.value)
    case 'saved':
      return acceptMetadataSave(action.value)
    case 'use-incoming':
      return { ...current, value: current.source, conflict: false }
  }
}
