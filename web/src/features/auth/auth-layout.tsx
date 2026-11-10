import { PublicFooter, PublicHeader, publicColumn } from '@/components/public-page'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { LockKeyhole } from 'lucide-react'
import type { ReactNode } from 'react'
import type { AuthAction } from './auth-failure-state'

const AUTH_PAGES = {
  'sign-in': {
    form: 'sign in',
    headline: 'Sign in to yours.',
    lede: 'Pick up your repositories, requests, and CLI sessions where you left them.',
    switchPrompt: 'New here?',
    switchLabel: 'Create an account',
    switchTo: '/sign-up/$',
  },
  'sign-up': {
    form: 'create account',
    headline: 'Start with yours.',
    lede: 'Share part of a repository and keep the rest private.',
    switchPrompt: 'Have an account?',
    switchLabel: 'Sign in',
    switchTo: '/sign-in/$',
  },
} as const

export function AuthLayout({ action, children }: { action: AuthAction; children: ReactNode }) {
  const page = AUTH_PAGES[action]
  return (
    <div className="flex min-h-dvh flex-col bg-background text-foreground">
      <div className={publicColumn}>
        <PublicHeader>
          <Link className="text-sm hover:text-success-strong" params={{ _splat: '' }} search to={page.switchTo}>
            <span className="text-muted-foreground max-[421px]:hidden">{page.switchPrompt} </span>
            {page.switchLabel}
          </Link>
        </PublicHeader>
      </div>
      <main className={cn(publicColumn, 'grid flex-1 content-center items-center gap-10 py-10 lg:grid-cols-[minmax(0,1fr)_400px] lg:gap-24 lg:pb-16')}>
        <div className="min-w-0">
          <h1 className="text-[clamp(40px,5.6vw,72px)] leading-[.98] font-medium tracking-[-.058em]">
            <span className="block">One repository.</span>
            <span className="block text-muted-foreground">{page.headline}</span>
          </h1>
          <p className="mt-6 max-w-[36ch] text-[17px] leading-[1.6] text-muted-foreground max-lg:hidden">{page.lede}</p>
        </div>
        <section aria-label={page.form} className="w-full min-w-0 max-w-[400px] lg:justify-self-end">
          <div className="flex h-11 items-center gap-2.5 border-b border-foreground font-mono text-[13px]">
            <LockKeyhole aria-hidden className="size-4 stroke-[1.6] text-muted-foreground" />
            {page.form}
          </div>
          <div className="pt-6">{children}</div>
        </section>
      </main>
      <div className={publicColumn}>
        <PublicFooter />
      </div>
    </div>
  )
}
