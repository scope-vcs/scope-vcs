import { Button } from '@/components/ui/button'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { Link } from '@tanstack/react-router'
import type { ReactNode } from 'react'

const SETTINGS_SECTIONS = [
  { id: 'profile', label: 'Profile' },
  { id: 'theme', label: 'Theme' },
  { id: 'cli-access', label: 'CLI access' },
  { id: 'delete-account', label: 'Delete account' },
] as const

type SettingsSectionId = (typeof SETTINGS_SECTIONS)[number]['id']

export function SettingsLayout({ children }: { children: ReactNode }) {
  return (
    <div className="grid gap-8 md:grid-cols-[180px_minmax(0,1fr)] md:gap-12">
      <nav aria-label="Settings sections" className="md:sticky md:top-10 md:self-start">
        <h1 className="text-xl font-semibold tracking-[-0.015em]">Settings</h1>
        <ul className="scrollbar-none -mx-2 mt-4 flex overflow-x-auto text-sm md:mx-0 md:flex-col md:gap-0.5 md:overflow-visible">
          {SETTINGS_SECTIONS.map((section) => (
            <li className={section.id === 'delete-account' ? 'shrink-0 md:mt-3' : 'shrink-0'} key={section.id}>
              <Link
                className="block whitespace-nowrap rounded-md px-2 py-1.5 md:px-2.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
                hash={section.id}
                to="/settings"
              >
                {section.label}
              </Link>
            </li>
          ))}
        </ul>
      </nav>
      <div className="min-w-0 divide-y divide-border">{children}</div>
    </div>
  )
}

export function SettingsSection({
  action,
  children,
  description,
  id,
  title,
}: {
  action?: ReactNode
  children?: ReactNode
  description?: ReactNode
  id: SettingsSectionId
  title: string
}) {
  return (
    <section aria-labelledby={`${id}-title`} className="scroll-mt-10 py-7 first:pt-0 last:pb-0" id={id}>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <h2 className="text-[15px] font-semibold leading-6" id={`${id}-title`}>{title}</h2>
          {description && (
            <p className="mt-0.5 max-w-[60ch] text-sm leading-5 text-muted-foreground">{description}</p>
          )}
        </div>
        {action}
      </div>
      {children && <div className="mt-4">{children}</div>}
    </section>
  )
}

export function CliAccessSection({ action, children }: { action: ReactNode; children: ReactNode }) {
  return (
    <SettingsSection
      action={action}
      description="Sessions from scope login and scope init. A login command signs in an agent, remote shell, or another terminal once."
      id="cli-access"
      title="CLI access"
    >
      {children}
    </SettingsSection>
  )
}

export function DeleteAccountRow({ onDelete }: { onDelete?: () => void }) {
  return (
    <SettingsSection
      action={(
        <Button disabled={!onDelete} onClick={onDelete} size="sm" type="button" variant="destructive">
          Delete account…
        </Button>
      )}
      description="Permanently deletes your account, CLI sessions, and the repositories you own. Your requests and discussions in other repositories stay and show as a deleted user."
      id="delete-account"
      title="Delete account"
    />
  )
}

export const SESSION_ROW_CLASS = 'flex flex-col gap-3 py-3 sm:flex-row sm:items-center sm:justify-between'

export function CliSessionListSkeleton() {
  return (
    <ul className="divide-y divide-border border-t border-border">
      {(['medium', 'long'] as const).map((length) => (
        <li className={SESSION_ROW_CLASS} key={length}>
          <div className="min-w-0">
            <TextSkeleton length={length} />
            <TextSkeleton className="mt-1" length="long" size="meta" />
          </div>
          <BlockSkeleton className="h-8 w-16 shrink-0" />
        </li>
      ))}
    </ul>
  )
}
