import { forceSignedOut } from '@/auth-mode'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { AuthLayout } from './auth-layout'
import { AuthSurface } from './auth-loading-state'

export type AuthAction = 'sign-in' | 'sign-up'

const AUTH_ACTION_LABELS: Record<AuthAction, string> = {
  'sign-in': 'Sign in',
  'sign-up': 'Sign up',
}

export function AuthFailureState({ action }: { action: AuthAction }) {
  const label = AUTH_ACTION_LABELS[action]
  return (
    <>
      <div className="border-t border-border pt-5" role="alert">
        <h2 className="text-sm font-semibold">{label} unavailable</h2>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">
          {forceSignedOut
            ? `${label} is disabled in this preview. You can still browse public repositories.`
            : `The ${label.toLowerCase()} form could not load. Try again to continue.`}
        </p>
      </div>
      <div className="mt-4 flex flex-wrap gap-3">
        {!forceSignedOut && (
          <Button onClick={() => window.location.reload()}>Try again</Button>
        )}
        <Button asChild variant="secondary">
          <Link to="/">Back to Scope</Link>
        </Button>
      </div>
    </>
  )
}

export function AuthRouteError({ action }: { action: AuthAction }) {
  return (
    <AuthLayout>
      <AuthSurface
        description="continue to your Scope account"
        title={`${AUTH_ACTION_LABELS[action]} couldn’t start`}
      >
        <AuthFailureState action={action} />
      </AuthSurface>
    </AuthLayout>
  )
}
