import type { ConnectRepoGitHubInput, GitHubSetupInput, RepoParams } from '@/api/types'
import type {
  GitHubAuthorizeResponse,
  GitHubConnectionResponse,
  GitHubSetupResponse,
} from '@/api/types.generated'
import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { BlockSkeleton } from '@/components/ui/skeleton'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { SignInButton, useAuth } from '@clerk/tanstack-react-start'
import { Link } from '@tanstack/react-router'
import { ExternalLink, LoaderCircle, LogIn, Plug, RefreshCw } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import {
  RUN_IMPORT_COUNT_HINT,
  parseRunImportCount,
} from '../repo-detail/repo-github-run-import-model'
import { githubSetupStep, type GitHubSetupSearch } from './github-setup-model'

type SetupState =
  | { kind: 'starting' }
  | { kind: 'pending'; setup: GitHubSetupResponse | null }
  | { kind: 'choosing'; setup: GitHubSetupResponse; selected: number | null }
  | { kind: 'error'; message: string; setup: GitHubSetupResponse | null }
  | { kind: 'declined' }
  | { kind: 'incomplete' }

type SetupActions = {
  completeSetup: (input: GitHubSetupInput) => Promise<GitHubSetupResponse>
  connect: (input: ConnectRepoGitHubInput) => Promise<GitHubConnectionResponse>
  onConnected: (setup: GitHubSetupResponse) => Promise<void>
  rememberPendingTarget: (target: RepoParams) => void
  startAuthorization: (target: RepoParams) => Promise<GitHubAuthorizeResponse>
  takePendingTarget: () => RepoParams | null
}

export function GitHubSetupView({ search, ...actions }: SetupActions & { search: GitHubSetupSearch }) {
  return (
    <AppShell header={() => <ApplicationTopbar contextLabel="GitHub" />}>
      <PageContent>
        <GitHubSetup actions={actions} search={search} />
      </PageContent>
    </AppShell>
  )
}

function GitHubSetup({ actions, search }: { actions: SetupActions; search: GitHubSetupSearch }) {
  const { isLoaded, isSignedIn } = useAuth()
  const [state, setState] = useState<SetupState>({ kind: 'starting' })
  const started = useRef(false)

  function authorize(target: RepoParams, setup: GitHubSetupResponse | null) {
    setState({ kind: 'pending', setup })
    startAuthorization(actions, target).catch((error: unknown) => {
      setState({ kind: 'error', message: resourceErrorMessage(error, 'GitHub could not be opened.'), setup })
    })
  }

  async function connectRepository(setup: GitHubSetupResponse, choice: RepositoryConnection) {
    setState({ kind: 'pending', setup })
    try {
      await actions.connect({
        owner: setup.owner_handle,
        repo: setup.repo_name,
        grant: setup.grant,
        github_repository_id: choice.githubRepositoryId,
        acknowledge_public: choice.acknowledgePublic,
        run_import_count: choice.runImportCount,
      })
      await actions.onConnected(setup)
    } catch (error) {
      setState({ kind: 'error', message: resourceErrorMessage(error, 'Connecting failed. Try again.'), setup })
    }
  }

  useEffect(() => {
    if (!isLoaded || !isSignedIn || started.current) return
    started.current = true
    const step = githubSetupStep(search, actions.takePendingTarget())
    if (step.kind === 'declined' || step.kind === 'incomplete') {
      setState({ kind: step.kind })
    } else if (step.kind === 'resume') {
      setState({ kind: 'pending', setup: null })
      startAuthorization(actions, step.target).catch((error: unknown) => {
        setState({ kind: 'error', message: resourceErrorMessage(error, 'GitHub could not be opened.'), setup: null })
      })
    } else {
      setState({ kind: 'pending', setup: null })
      actions.completeSetup({ code: step.code, state: step.state }).then(
        (setup) => setState({
          kind: 'choosing',
          setup,
          selected: setup.repositories.length === 1 ? setup.repositories[0].id : null,
        }),
        (error: unknown) => setState({
          kind: 'error',
          message: resourceErrorMessage(error, 'GitHub setup failed.'),
          setup: null,
        }),
      )
    }
  }, [actions, isLoaded, isSignedIn, search])

  if (state.kind === 'declined') {
    return (
      <Closed
        description="GitHub did not authorize the Scope GitHub App, so nothing was connected. Start again from repository settings."
        title="GitHub authorization was cancelled"
      />
    )
  }
  if (state.kind === 'incomplete') {
    return (
      <Closed
        description="Start connecting from the CI section of repository settings."
        title="Nothing to connect"
      />
    )
  }

  const setup = state.kind === 'starting' || !('setup' in state) ? null : state.setup
  const pending = state.kind === 'pending'

  return (
    <>
      <PageHeader
        description={
          setup
            ? `Choose the GitHub repository whose workflows check requests in ${setup.owner_handle}/${setup.repo_name}.`
            : 'Finishing the connection with GitHub.'
        }
        title="Connect GitHub"
      />

      {state.kind === 'error' && (
        <PageErrorAlert title={setup ? 'Repository not connected' : 'GitHub setup failed'}>
          {state.message}
        </PageErrorAlert>
      )}

      <div className="mt-6 space-y-6 text-sm">
        {isLoaded && !isSignedIn && (
          <div className="space-y-3">
            <p className="leading-5 text-muted-foreground">Sign in to Scope to finish connecting.</p>
            <SignInButton mode="modal">
              <Button size="sm" type="button">
                <LogIn className="size-3.5" />
                <span>Sign in</span>
              </Button>
            </SignInButton>
          </div>
        )}

        {!setup && isSignedIn && state.kind !== 'error' && (
          <div className="flex items-center gap-2 text-muted-foreground">
            <LoaderCircle className="size-3.5 animate-spin" />
            <span>Checking which repositories you can connect</span>
          </div>
        )}
        {!isLoaded && <BlockSkeleton className="h-8 w-24" />}

        {setup && setup.repositories.length > 0 && (
          <RepositoryChoice
            onChoose={(selected) => setState({ kind: 'choosing', setup, selected })}
            onConnect={(choice) => void connectRepository(setup, choice)}
            pending={pending}
            selected={state.kind === 'choosing' ? state.selected : null}
            setup={setup}
          />
        )}

        {setup && (
          <InstallPrompt
            empty={setup.repositories.length === 0}
            onCheckAgain={() => authorize({ owner: setup.owner_handle, repo: setup.repo_name }, setup)}
            onInstall={() => actions.rememberPendingTarget({ owner: setup.owner_handle, repo: setup.repo_name })}
            pending={pending}
            setup={setup}
          />
        )}
      </div>
    </>
  )
}

async function startAuthorization(actions: SetupActions, target: RepoParams) {
  const { authorize_url } = await actions.startAuthorization(target)
  window.location.assign(authorize_url)
}

type RepositoryConnection = {
  githubRepositoryId: number
  acknowledgePublic: boolean
  runImportCount: number
}

function RepositoryChoice({
  onChoose,
  onConnect,
  pending,
  selected,
  setup,
}: {
  onChoose: (id: number) => void
  onConnect: (choice: RepositoryConnection) => void
  pending: boolean
  selected: number | null
  setup: GitHubSetupResponse
}) {
  const [acknowledged, setAcknowledged] = useState(false)
  const [runImportDraft, setRunImportDraft] = useState(String(setup.run_import_count))
  const runImportCount = parseRunImportCount(runImportDraft)
  const chosen = setup.repositories.find((repository) => repository.id === selected)
  const isPublic = chosen ? !chosen.private : false
  return (
    <form
      className="space-y-4"
      onSubmit={(event) => {
        event.preventDefault()
        if (selected === null || runImportCount === null) return
        onConnect({
          githubRepositoryId: selected,
          acknowledgePublic: isPublic && acknowledged,
          runImportCount,
        })
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
      {isPublic && (
        <label className="flex items-start gap-3 leading-5">
          <input
            checked={acknowledged}
            className="mt-1"
            disabled={pending}
            onChange={(event) => setAcknowledged(event.target.checked)}
            type="checkbox"
          />
          <span>
            {chosen?.full_name} is public. Everything Scope pushes there, including private
            requests and private files, becomes public on GitHub.
          </span>
        </label>
      )}
      <div className="space-y-2">
        <div className="flex flex-wrap items-center gap-3">
          <label className="leading-5" htmlFor="github-run-import-count">Recent runs to import</label>
          <Input
            aria-describedby="github-run-import-count-help"
            aria-invalid={runImportCount === null}
            className="h-8 w-24"
            disabled={pending}
            id="github-run-import-count"
            inputMode="numeric"
            max={1000}
            min={0}
            onChange={(event) => setRunImportDraft(event.target.value)}
            type="number"
            value={runImportDraft}
          />
        </div>
        <p
          className={runImportCount === null ? 'leading-5 text-danger-strong' : 'leading-5 text-muted-foreground'}
          id="github-run-import-count-help"
        >
          {runImportCount === null
            ? RUN_IMPORT_COUNT_HINT
            : 'Scope reads this many of GitHub’s most recent workflow runs for the Runs page. 0 imports none.'}
        </p>
      </div>
      <div className="flex flex-wrap gap-2">
        <Button
          disabled={pending || selected === null || runImportCount === null || (isPublic && !acknowledged)}
          size="sm"
          type="submit"
        >
          {pending ? <LoaderCircle className="size-3.5 animate-spin" /> : <Plug className="size-3.5" />}
          <span>Connect repository</span>
        </Button>
        <Button asChild size="sm" variant="ghost">
          <Link params={{ owner: setup.owner_handle, repo: setup.repo_name }} to="/$owner/$repo/settings">
            Back to settings
          </Link>
        </Button>
      </div>
    </form>
  )
}

function InstallPrompt({
  empty,
  onCheckAgain,
  onInstall,
  pending,
  setup,
}: {
  empty: boolean
  onCheckAgain: () => void
  onInstall: () => void
  pending: boolean
  setup: GitHubSetupResponse
}) {
  return (
    <section className="space-y-3 border-t border-border pt-5">
      <p className="leading-5 text-muted-foreground">
        {empty
          ? 'Your GitHub account cannot push any repository the Scope GitHub App is installed on.'
          : "Don't see your repository? The app must be installed on it, and you need push access."}
      </p>
      <div className="flex flex-wrap gap-2">
        <Button asChild size="sm" variant={empty ? 'default' : 'secondary'}>
          <a href={setup.install_url} onClick={onInstall}>
            <ExternalLink className="size-3.5" />
            <span>Install the Scope GitHub App on your repository</span>
          </a>
        </Button>
        <Button disabled={pending} onClick={onCheckAgain} size="sm" type="button" variant="ghost">
          <RefreshCw className="size-3.5" />
          <span>Check again</span>
        </Button>
        {empty && (
          <Button asChild size="sm" variant="ghost">
            <Link params={{ owner: setup.owner_handle, repo: setup.repo_name }} to="/$owner/$repo/settings">
              Back to settings
            </Link>
          </Button>
        )}
      </div>
    </section>
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
