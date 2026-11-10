import { PublicFooter, PublicHeader, publicColumn } from '@/components/public-page'
import { cn } from '@/lib/utils'
import type { ReactNode } from 'react'
import type { AuthAction } from './auth-failure-state'

const AUTH_PAGES = {
  'sign-in': {
    title: 'Sign in to Scope',
    lede: 'Pick up your repositories, requests, and CLI sessions.',
  },
  'sign-up': {
    title: 'Create your Scope account',
    lede: 'Share part of a repository and keep the rest private.',
  },
} as const

export function AuthLayout({ action, children }: { action: AuthAction; children: ReactNode }) {
  const page = AUTH_PAGES[action]
  return (
    <div className="flex min-h-dvh flex-col bg-background text-foreground">
      <div className={publicColumn}>
        <PublicHeader />
      </div>
      <main className={cn(publicColumn, 'flex-1 pt-12 pb-16 sm:pt-[120px]')}>
        <section aria-labelledby="auth-title" className="mx-auto w-full max-w-[400px]">
          <h1 className="text-[30px] leading-[1.05] font-medium tracking-[-.045em] sm:text-4xl" id="auth-title">
            {page.title}
          </h1>
          <p className="mt-2.5 mb-8 text-[15px] leading-normal text-muted-foreground">{page.lede}</p>
          {children}
        </section>
      </main>
      <div className={publicColumn}>
        <PublicFooter />
      </div>
    </div>
  )
}
