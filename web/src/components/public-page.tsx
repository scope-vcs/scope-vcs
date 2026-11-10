import { ScopeLogo } from '@/components/scope-logo'
import { SiteLinks, ThemeSwitch } from '@/components/site-footer'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import type { ReactElement, ReactNode } from 'react'

export const publicColumn = 'mx-auto w-[calc(100%-64px)] max-w-[1120px] max-[521px]:w-[calc(100%-40px)]'

export function PublicHeader({ children, logoClassName }: { children: ReactNode; logoClassName?: string }): ReactElement {
  return (
    <header className="relative flex h-[76px] items-center justify-between">
      <Link aria-label="Scope home" to="/">
        <ScopeLogo className={logoClassName} />
      </Link>
      {children}
    </header>
  )
}

export function PublicFooter({
  children,
  label,
  logoClassName,
}: {
  children?: ReactNode
  label?: string
  logoClassName?: string
}): ReactElement {
  return (
    <footer className="relative flex min-h-24 flex-wrap items-center justify-between gap-5 text-[13px] text-muted-foreground max-[901px]:pb-8">
      <Link aria-label="Scope home" to="/">
        <ScopeLogo className={cn('h-5 opacity-65', logoClassName)} />
      </Link>
      {children}
      <div className="flex flex-wrap items-center gap-x-6 gap-y-3">
        <SiteLinks className="flex gap-6 [&_a:hover]:text-success-strong" label={label} />
        <ThemeSwitch />
      </div>
    </footer>
  )
}
