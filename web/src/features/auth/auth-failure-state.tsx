import { forceSignedOut } from '@/auth-mode'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { AuthLayout } from './auth-layout'
import { AuthSurface } from './auth-loading-state'

export function AuthFailureState({ title }: { title: string }) {
  return (
    <>
      <div className="border-t border-border pt-5" role="alert">
        <h2 className="text-sm font-semibold">{title}</h2>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">
          {forceSignedOut
            ? 'sign-in is disabled in this preview. You can still browse public repositories.'
            : 'the sign-in form could not load. Try again to continue.'}
        </p>
      </div>
      <div className="mt-4 flex flex-wrap gap-3">
        {!forceSignedOut && (
          <Button onClick={() => window.location.reload()}>try again</Button>
        )}
        <Button asChild variant="secondary">
          <Link to="/">back to scope</Link>
        </Button>
      </div>
    </>
  )
}

export function AuthRouteError() {
  return (
    <AuthLayout>
      <AuthSurface description="continue to your Scope account" title="sign-in couldn't start">
        <AuthFailureState title="sign-in unavailable" />
      </AuthSurface>
    </AuthLayout>
  )
}
