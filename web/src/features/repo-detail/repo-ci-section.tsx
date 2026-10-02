import type { GitHubConnectionResponse, GitHubAuthorizeResponse } from '@/api/types.generated'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { LoaderCircle, Plug, Unplug } from 'lucide-react'
import { useState } from 'react'
import { githubConnectionView, githubVisibilityView } from './repo-github-connection-model'
import { RepoGitHubSetupCheck } from './repo-github-setup-check'
import { RepoRequiredChecks } from './repo-required-checks'
import { CiSection } from './repo-settings-sections'

/** `github` is `null` until it loads. */
export function RepoCiSection({
  confirmPublic,
  disconnect,
  github,
  setRequiredChecks,
  startAuthorization,
  startSetupCheck,
}: {
  confirmPublic: () => Promise<GitHubConnectionResponse>
  disconnect: () => Promise<GitHubConnectionResponse>
  github: GitHubConnectionResponse | null
  setRequiredChecks: (names: string[]) => Promise<GitHubConnectionResponse>
  startAuthorization: () => Promise<GitHubAuthorizeResponse>
  startSetupCheck: () => Promise<GitHubConnectionResponse>
}) {
  const [pending, setPending] = useState<'connect' | 'disconnect' | 'confirm' | null>(null)
  const [error, setError] = useState<{ title: string; message: string } | null>(null)

  async function connect() {
    setError(null)
    setPending('connect')
    try {
      const { authorize_url } = await startAuthorization()
      // Pending lasts until GitHub's authorization screen replaces this page.
      window.location.assign(authorize_url)
    } catch (cause) {
      setPending(null)
      setError({ title: 'GitHub could not be opened', message: resourceErrorMessage(cause, 'Try again.') })
    }
  }

  async function disconnectRepository() {
    setError(null)
    setPending('disconnect')
    try {
      await disconnect()
    } catch (cause) {
      setError({ title: 'Disconnect failed', message: resourceErrorMessage(cause, 'Try again.') })
    } finally {
      setPending(null)
    }
  }

  async function confirmPublicRepository() {
    setError(null)
    setPending('confirm')
    try {
      await confirmPublic()
    } catch (cause) {
      setError({ title: 'Confirmation failed', message: resourceErrorMessage(cause, 'Try again.') })
    } finally {
      setPending(null)
    }
  }

  if (!github) {
    return (
      <CiSection>
        <TextSkeleton length="long" />
      </CiSection>
    )
  }

  const view = githubConnectionView(github)
  const visibility = githubVisibilityView(github)
  const spinner = <LoaderCircle className="size-3.5 animate-spin" />
  const connectButton = (label: string) => (
    <Button disabled={pending !== null} onClick={() => void connect()} size="sm" type="button">
      {pending === 'connect' ? spinner : <Plug className="size-3.5" />}
      <span>{label}</span>
    </Button>
  )
  const repositoryLink = (name: string, url: string) => (
    <a className="font-medium text-foreground underline-offset-4 hover:underline" href={url} rel="noreferrer" target="_blank">
      {name}
    </a>
  )

  return (
    <CiSection>
      <div className="space-y-3 text-sm">
        {view.kind === 'unconfigured' && (
          <p className="leading-5 text-muted-foreground">GitHub is not configured on this server.</p>
        )}

        {view.kind === 'not_connected' && (
          <div className="flex flex-wrap items-center justify-between gap-3">
            <p className="leading-5 text-muted-foreground">Not connected to GitHub.</p>
            {connectButton('Connect GitHub')}
          </div>
        )}

        {view.kind === 'connected' && (
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="min-w-0">
              <div className="truncate leading-5">Connected to {repositoryLink(view.name, view.url)}</div>
              <div className="leading-5 text-muted-foreground">{view.detail}</div>
            </div>
            <Button
              disabled={pending !== null}
              onClick={() => void disconnectRepository()}
              size="sm"
              type="button"
              variant="secondary"
            >
              {pending === 'disconnect' ? spinner : <Unplug className="size-3.5" />}
              <span>Disconnect</span>
            </Button>
          </div>
        )}

        {view.kind === 'disconnected' && (
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="min-w-0">
              <div className="truncate leading-5">Disconnected from {repositoryLink(view.name, view.url)}</div>
              <div className="leading-5 text-muted-foreground">{view.reason}</div>
            </div>
            {connectButton('Reconnect')}
          </div>
        )}

        {visibility?.kind === 'public' && (
          <p className="leading-5 text-muted-foreground">
            Public on GitHub: everything Scope pushes here is public.
          </p>
        )}

        {visibility?.kind === 'unconfirmed' && (
          <div className="flex flex-wrap items-center justify-between gap-3">
            <p className="min-w-0 flex-1 leading-5 text-warning-strong" role="note">
              This GitHub repository became public, so Scope stopped sending private requests there.
              {visibility.canConfirm
                ? ' Allowing them makes their changes and private files public on GitHub.'
                : ' A maintainer who can change file visibility can allow them.'}
            </p>
            {visibility.canConfirm && (
              <Button
                disabled={pending !== null}
                onClick={() => void confirmPublicRepository()}
                size="sm"
                type="button"
                variant="secondary"
              >
                {pending === 'confirm' && spinner}
                <span>Allow private requests</span>
              </Button>
            )}
          </div>
        )}

        {view.kind === 'connected' && (
          <RepoGitHubSetupCheck
            github={github}
            requireCheck={(name) => setRequiredChecks([...github.required_checks, name])}
            startTest={startSetupCheck}
          />
        )}

        {(view.kind === 'connected' || view.kind === 'disconnected') && (
          <RepoRequiredChecks names={github.required_checks} save={setRequiredChecks} />
        )}

        {error && <PageErrorAlert className="mt-0" title={error.title}>{error.message}</PageErrorAlert>}
      </div>
    </CiSection>
  )
}
