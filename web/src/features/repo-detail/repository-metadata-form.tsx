import type { UpdateRepoMetadataInput } from '@/api/types'
import type { RepoSummaryResponse } from '@/api/types.generated'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useState, type FormEvent } from 'react'

type Metadata = Pick<UpdateRepoMetadataInput, 'description' | 'website_url'>
type SaveState =
  | { status: 'idle' | 'saving' | 'saved' }
  | { status: 'failed'; message: string }

export function RepositoryMetadataForm({
  repo,
  save,
}: {
  repo: RepoSummaryResponse
  save: (metadata: Metadata) => Promise<RepoSummaryResponse>
}) {
  const [state, setState] = useState<SaveState>({ status: 'idle' })

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (state.status === 'saving') return
    const data = new FormData(event.currentTarget)
    setState({ status: 'saving' })
    try {
      await save({
        description: String(data.get('description') ?? ''),
        website_url: String(data.get('website_url') ?? ''),
      })
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
          key={`${repo.id}\0${repo.description}\0${repo.website_url}`}
          onChange={() => {
            if (state.status !== 'saving') setState({ status: 'idle' })
          }}
          onSubmit={submit}
        >
          <fieldset className="max-w-xl space-y-4" disabled={state.status === 'saving'}>
            <div>
              <label className="mb-1.5 block text-sm font-medium" htmlFor="repo-description">
                Description
              </label>
              <Input
                defaultValue={repo.description ?? ''}
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
                defaultValue={repo.website_url ?? ''}
                id="repo-website"
                maxLength={2048}
                name="website_url"
                placeholder="https://example.com"
                type="url"
              />
            </div>
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
