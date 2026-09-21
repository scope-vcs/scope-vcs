import type { RepoInviteTokenInput } from '@/api/types'
import type {
  AcceptRepositoryInviteResponse,
  RepositoryInviteLandingResponse,
} from '@/api/types.generated'
import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { MemberAccessSummary } from '@/features/repo-detail/repo-members-section'
import { formatUnixDateUtc } from '@/lib/date-format'
import { Link, useNavigate, useRouter } from '@tanstack/react-router'
import { useAuth, useClerk } from '@clerk/tanstack-react-start'
import { Check, LoaderCircle } from 'lucide-react'
import { useEffect, useRef, useState, type ReactNode } from 'react'

type OpenInvite = Extract<RepositoryInviteLandingResponse, { status: 'open' }>

export function InvitePage({
  acceptInvite,
  invite,
  token,
}: {
  acceptInvite: (
    input: RepoInviteTokenInput,
  ) => Promise<AcceptRepositoryInviteResponse>
  invite: RepositoryInviteLandingResponse
  token: string
}) {
  useSignedInLanding(invite)

  return (
    <AppShell
      header={() => <ApplicationTopbar contextLabel="Repository invite" />}
    >
      <PageContent>
        <InviteLanding acceptInvite={acceptInvite} invite={invite} token={token} />
      </PageContent>
    </AppShell>
  )
}

/**
 * The server can render before Clerk has refreshed an idle session, and then
 * answers as if nobody were signed in. Once the browser knows better, load the
 * landing again, a single time, so it describes the actual viewer.
 */
function useSignedInLanding(invite: RepositoryInviteLandingResponse) {
  const { isLoaded, isSignedIn } = useAuth()
  const router = useRouter()
  const reloaded = useRef(false)
  const answeredWithoutViewer =
    invite.status === 'used' ||
    (invite.status === 'open' && invite.viewer === 'signed_out')

  useEffect(() => {
    if (!isLoaded || !isSignedIn || !answeredWithoutViewer || reloaded.current) return
    reloaded.current = true
    void router.invalidate()
  }, [answeredWithoutViewer, isLoaded, isSignedIn, router])
}

function InviteLanding({
  acceptInvite,
  invite,
  token,
}: {
  acceptInvite: (
    input: RepoInviteTokenInput,
  ) => Promise<AcceptRepositoryInviteResponse>
  invite: RepositoryInviteLandingResponse
  token: string
}) {
  switch (invite.status) {
    case 'open':
      return <OpenInviteLanding acceptInvite={acceptInvite} invite={invite} token={token} />
    case 'member':
      return (
        <ClosedInvite
          description="Your access is ready."
          title={`You're a member of ${invite.owner_handle}/${invite.repo_name}`}
        >
          <Button asChild>
            <Link
              params={{ owner: invite.owner_handle, repo: invite.repo_name }}
              to="/$owner/$repo"
            >
              Open repository
            </Link>
          </Button>
        </ClosedInvite>
      )
    case 'expired':
      return (
        <ClosedInvite
          description={`Ask @${invite.owner_handle} to send a new invite. This link can no longer be accepted.`}
          title={`This invite to ${invite.owner_handle}/${invite.repo_name} has expired`}
        />
      )
    case 'revoked':
      return (
        <ClosedInvite
          description="The owner withdrew this invite. Contact the person who shared it if you still need access."
          title="This invite was revoked"
        />
      )
    case 'access_removed':
      return (
        <ClosedInvite
          description="This invite was already used. Ask the repository owner for a new invite if you need to rejoin."
          title="You no longer have member access"
        />
      )
    case 'used':
      return (
        <ClosedInvite
          description="If you accepted it, sign in with that account to open the repository."
          title="This invite was already used"
        />
      )
    case 'invalid':
      return (
        <ClosedInvite
          description="Check that you copied the whole link, or ask the sender for a new invite."
          title="This link doesn't work"
        />
      )
  }
}

function ClosedInvite({
  children,
  description,
  title,
}: {
  children?: ReactNode
  description: string
  title: string
}) {
  return (
    <>
      <PageHeader description={description} title={title} />
      <div className="mt-6 flex flex-wrap gap-2">
        {children ?? (
          <Button asChild variant="secondary">
            <Link to="/">Go to your repositories</Link>
          </Button>
        )}
      </div>
    </>
  )
}

function OpenInviteLanding({
  acceptInvite,
  invite,
  token,
}: {
  acceptInvite: (
    input: RepoInviteTokenInput,
  ) => Promise<AcceptRepositoryInviteResponse>
  invite: OpenInvite
  token: string
}) {
  return (
    <>
      <PageHeader
        description={`@${invite.owner_handle} invited ${invite.invited_email} to become a member.`}
        title={`Join ${invite.owner_handle}/${invite.repo_name}`}
      />

      <div className="mt-6 divide-y divide-border">
        <section className="grid gap-4 py-5 md:grid-cols-[220px_minmax(0,1fr)]">
          <div>
            <div className="text-sm font-semibold leading-5">Access</div>
            <p className="mt-1 text-sm leading-5 text-muted-foreground">
              Access starts after you accept.
            </p>
          </div>
          <MemberAccessSummary permissions={invite.permissions} />
        </section>

        <section className="grid gap-4 py-5 md:grid-cols-[220px_minmax(0,1fr)]">
          <div>
            <div className="text-sm font-semibold leading-5">Continue</div>
            <p className="mt-1 text-sm leading-5 text-muted-foreground">
              Expires {formatUnixDateUtc(invite.expires_at_unix)} UTC.
            </p>
          </div>
          <InviteViewerActions acceptInvite={acceptInvite} invite={invite} token={token} />
        </section>
      </div>
    </>
  )
}

function InviteViewerActions({
  acceptInvite,
  invite,
  token,
}: {
  acceptInvite: (
    input: RepoInviteTokenInput,
  ) => Promise<AcceptRepositoryInviteResponse>
  invite: OpenInvite
  token: string
}) {
  const clerk = useClerk()
  const navigate = useNavigate()
  const router = useRouter()
  const [acceptError, setAcceptError] = useState<string | null>(null)
  const [pending, setPending] = useState(false)
  const returnPath = `/invites/${encodeURIComponent(token)}`
  const authSearch = { redirect_url: returnPath }

  async function onAccept() {
    if (pending) return

    setAcceptError(null)
    setPending(true)
    try {
      const accepted = await acceptInvite({ token })
      await navigate({
        params: {
          owner: accepted.repo.owner_handle,
          repo: accepted.repo.name,
        },
        to: '/$owner/$repo',
      })
    } catch (error) {
      setAcceptError(error instanceof Error ? error.message : 'Invite could not be accepted.')
      // The invite may have been revoked or used meanwhile; show what it is now.
      await router.invalidate()
    } finally {
      setPending(false)
    }
  }

  async function switchAccount() {
    if (pending) return

    setPending(true)
    try {
      await clerk.signOut({
        redirectUrl: `/sign-in?redirect_url=${encodeURIComponent(returnPath)}`,
      })
    } finally {
      setPending(false)
    }
  }

  const spinner = pending && <LoaderCircle className="size-3.5 animate-spin" />

  return (
    <div className="space-y-3 text-sm">
      {invite.viewer === 'ready' && (
        <>
          <p className="leading-5 text-muted-foreground">
            Accepting as <span className="font-medium text-foreground">{invite.viewer_email}</span>
          </p>
          <div className="flex flex-wrap gap-2">
            <Button disabled={pending} onClick={() => void onAccept()} type="button">
              {spinner || <Check className="size-3.5" />}
              <span>Accept invite</span>
            </Button>
            <Button
              disabled={pending}
              onClick={() => void switchAccount()}
              type="button"
              variant="ghost"
            >
              Use another account
            </Button>
          </div>
        </>
      )}

      {invite.viewer === 'signed_out' && (
        <>
          <p className="leading-5 text-muted-foreground">
            Sign in or create an account with{' '}
            <span className="font-medium text-foreground">{invite.invited_email}</span>.
            You'll come back here to accept.
          </p>
          <div className="flex flex-wrap gap-2">
            <Button asChild>
              <Link params={{ _splat: '' }} search={authSearch} to="/sign-up/$">
                Create account
              </Link>
            </Button>
            <Button asChild variant="secondary">
              <Link params={{ _splat: '' }} search={authSearch} to="/sign-in/$">
                Sign in
              </Link>
            </Button>
          </div>
        </>
      )}

      {invite.viewer === 'wrong_account' && (
        <>
          <p className="leading-5 text-muted-foreground">
            This invite is for{' '}
            <span className="font-medium text-foreground">{invite.invited_email}</span>.
            You're signed in as{' '}
            <span className="font-medium text-foreground">{invite.viewer_email}</span>.
          </p>
          <div className="flex flex-wrap gap-2">
            <Button disabled={pending} onClick={() => void switchAccount()} type="button">
              {spinner}
              <span>Switch account</span>
            </Button>
          </div>
        </>
      )}

      {invite.viewer === 'email_unverified' && (
        <>
          <p className="leading-5 text-muted-foreground">
            Verify{' '}
            <span className="font-medium text-foreground">{invite.invited_email}</span>{' '}
            in your account, then open this link again.
          </p>
          <div className="flex flex-wrap gap-2">
            <Button asChild>
              <Link to="/account">Verify email</Link>
            </Button>
          </div>
        </>
      )}

      {acceptError && (
        <PageErrorAlert title="Invite could not be accepted">
          {acceptError}
        </PageErrorAlert>
      )}
    </div>
  )
}
