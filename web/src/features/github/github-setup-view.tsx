import type { ConnectRepoGitHubInput, GitHubSetupInput } from '@/api/types'
import type {
  GitHubConnectionResponse,
  GitHubSetupResponse,
} from '@/api/types.generated'
import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { BlockSkeleton } from '@/components/ui/skeleton'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { SignInButton, useAuth } from '@clerk/tanstack-react-start'
import { Link } from '@tanstack/react-router'
import { LoaderCircle, LogIn, Plug } from 'lucide-react'
import { useState } from 'react'

/** What GitHub's install screen sent back. */
export type GitHubSetupSearch = {
  code?: string
  installation_id?: number
  setup_action?: string
  state?: string
}

type SetupState =
  | { kind: 'idle' }
  | { kind: 'pending' }
  | { kind: 'choosing'; setup: GitHubSetupResponse; selected: number | null }
  | { kind: 'error'; message: string; setup: GitHubSetupResponse | null }

/**
 * Finishes connecting after GitHub's install screen. GitHub's code can be
 * used once, so nothing is sent until the signed-in maintainer continues.
 * With one repository to choose, continuing connects it straight away.
 */
export function GitHubSetupView({
  completeSetup,
  connect,
  onConnected,
  search,
}: {
  completeSetup: (input: GitHubSetupInput) => Promise<GitHubSetupResponse>
  connect: (input: ConnectRepoGitHubInput) => Promise<GitHubConnectionResponse>
  onConnected: (setup: GitHubSetupResponse) => Promise<void>
  search: GitHubSetupSearch
}) {
  return (
    <AppShell header={() => <ApplicationTopbar contextLabel="GitHub" />}>
      <PageContent>
        <GitHubSetup
          completeSetup={completeSetup}
          connect={connect}
          onConnected={onConnected}
          search={search}
        />
      </PageContent>
    </AppShell>
  )
}

function GitHubSetup({
  completeSetup,
  connect,
  onConnected,
  search,
}: {
  completeSetup: (input: GitHubSetupInput) => Promise<GitHubSetupResponse>
  connect: (input: ConnectRepoGitHubInput) => Promise<GitHubConnectionResponse>
  onConnected: (setup: GitHubSetupResponse) => Promise<void>
  search: GitHubSetupSearch
}) {
  const [state, setState] = useState<SetupState>({ kind: 'idle' })

  if (search.setup_action === 'request') {
    return (
      <Closed
        description="An owner of the GitHub account must approve installing the Scope GitHub App. Once they do, connect again from repository settings."
        title="Installation requested"
      />
    )
  }
  const { code, installation_id, state: installState } = search
  if (!code || !installation_id || !installState) {
    return (
      <Closed
        description="GitHub did not send everything needed to finish connecting. Start again from repository settings."
        title="This setup link doesn't work"
      />
    )
  }

  async function connectRepository(setup: GitHubSetupResponse, githubRepositoryId: number) {
    setState({ kind: 'pending' })
    try {
      await connect({
        owner: setup.owner_handle,
        repo: setup.repo_name,
        grant: setup.grant,
        github_repository_id: githubRepositoryId,
      })
      await onConnected(setup)
    } catch (error) {
      setState({ kind: 'error', message: resourceErrorMessage(error, 'Connecting failed. Try again.'), setup })
    }
  }

  async function continueSetup() {
    if (!code || !installation_id || !installState) return
    setState({ kind: 'pending' })
    let setup: GitHubSetupResponse
    try {
      setup = await completeSetup({ code, installation_id, state: installState })
    } catch (error) {
      setState({ kind: 'error', message: resourceErrorMessage(error, 'GitHub setup failed.'), setup: null })
      return
    }
    if (setup.repositories.length === 1) {
      await connectRepository(setup, setup.repositories[0].id)
    } else {
      setState({ kind: 'choosing', setup, selected: null })
    }
  }

  const setup = state.kind === 'choosing' || state.kind === 'error' ? state.setup : null
  const pending = state.kind === 'pending'

  return (
    <>
      <PageHeader
        description={
          setup
            ? `Choose the GitHub repository whose workflows check requests in ${setup.owner_handle}/${setup.repo_name}.`
            : 'Finish connecting the repository you chose on GitHub.'
        }
        title="Connect GitHub"
      />

      {state.kind === 'error' && (
        <PageErrorAlert title={setup ? 'Repository not connected' : 'GitHub setup failed'}>
          {state.message}
        </PageErrorAlert>
      )}

      <div className="mt-6 space-y-4 text-sm">
        {!setup && (
          <SetupAction onContinue={() => void continueSetup()} pending={pending} />
        )}

        {setup && setup.repositories.length === 0 && (
          <p className="leading-5 text-muted-foreground">
            The installation has no repositories your GitHub account can access. Add the repository to the
            installation on GitHub, then connect again from repository settings.
          </p>
        )}

        {setup && setup.repositories.length > 0 && (
          <RepositoryChoice
            onChoose={(selected) => setState({ kind: 'choosing', setup, selected })}
            onConnect={(id) => void connectRepository(setup, id)}
            pending={pending}
            selected={state.kind === 'choosing' ? state.selected : null}
            setup={setup}
          />
        )}

        {setup && (
          <Button asChild variant="ghost" size="sm">
            <Link params={{ owner: setup.owner_handle, repo: setup.repo_name }} to="/$owner/$repo/settings">
              Back to settings
            </Link>
          </Button>
        )}
      </div>
    </>
  )
}

function SetupAction({ onContinue, pending }: { onContinue: () => void; pending: boolean }) {
  const { isLoaded, isSignedIn } = useAuth()
  if (!isLoaded) return <BlockSkeleton className="h-8 w-24" />
  if (!isSignedIn) {
    return (
      <div className="space-y-3">
        <p className="leading-5 text-muted-foreground">Sign in to Scope to finish connecting.</p>
        <SignInButton mode="modal">
          <Button size="sm" type="button">
            <LogIn className="size-3.5" />
            <span>Sign in</span>
          </Button>
        </SignInButton>
      </div>
    )
  }
  return (
    <Button disabled={pending} onClick={onContinue} size="sm" type="button">
      {pending ? <LoaderCircle className="size-3.5 animate-spin" /> : <Plug className="size-3.5" />}
      <span>Continue</span>
    </Button>
  )
}

function RepositoryChoice({
  onChoose,
  onConnect,
  pending,
  selected,
  setup,
}: {
  onChoose: (id: number) => void
  onConnect: (id: number) => void
  pending: boolean
  selected: number | null
  setup: GitHubSetupResponse
}) {
  return (
    <form
      className="space-y-4"
      onSubmit={(event) => {
        event.preventDefault()
        if (selected !== null) onConnect(selected)
      }}
    >
      <fieldset className="divide-y divide-border border-y border-border">
        <legend className="sr-only">GitHub repository</legend>
        {setup.repositories.map((repository) => (
          <label className="flex cursor-pointer items-center gap-3 py-3" key={repository.id}>
            <input
              checked={selected === repository.id}
              disabled={pending}
              name="github-repository"
              onChange={() => onChoose(repository.id)}
              type="radio"
            />
            <span className="min-w-0 truncate font-medium">{repository.full_name}</span>
            <span className="text-muted-foreground">{repository.private ? 'Private' : 'Public'}</span>
          </label>
        ))}
      </fieldset>
      <Button disabled={pending || selected === null} size="sm" type="submit">
        {pending ? <LoaderCircle className="size-3.5 animate-spin" /> : <Plug className="size-3.5" />}
        <span>Connect repository</span>
      </Button>
    </form>
  )
}

function Closed({ description, title }: { description: string; title: string }) {
  return (
    <>
      <PageHeader description={description} title={title} />
      <div className="mt-6">
        <Button asChild variant="secondary">
          <Link to="/">Go to your repositories</Link>
        </Button>
      </div>
    </>
  )
}
