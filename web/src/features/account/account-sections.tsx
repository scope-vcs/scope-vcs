import { SectionRow, SectionRows } from '@/components/section-rows'
import { Button } from '@/components/ui/button'
import { BlockSkeleton, TextSkeleton } from '@/components/ui/skeleton'
import { KeyRound, Monitor, Trash2 } from 'lucide-react'
import type { ReactNode } from 'react'

// The account page's fixed sections, shared with its pending state.

export function CliLoginSection({ children }: { children: ReactNode }) {
  return (
    <SectionRow
      description="Create a short-lived command for agents, remote shells, or another terminal."
      icon={<KeyRound className="size-4" />}
      title="One-time CLI login"
    >
      {children}
    </SectionRow>
  )
}

export function CliSessionsSection({ children }: { children: ReactNode }) {
  return (
    <SectionRow
      description="Active sessions created by scope login or scope init."
      icon={<Monitor className="size-4" />}
      title="CLI sessions"
    >
      {children}
    </SectionRow>
  )
}

/** Disabled until `onDelete` is given, which needs the account's handle. */
export function AccountDangerZoneSection({ onDelete }: { onDelete?: () => void }) {
  return (
    <SectionRows>
      <SectionRow
        description="Permanently deletes your account, CLI sessions, and the repositories you own. Your requests and discussions in other repositories stay and show as a deleted user."
        icon={<Trash2 className="size-4" />}
        title="Danger zone"
      >
        <Button
          disabled={!onDelete}
          onClick={onDelete}
          size="sm"
          type="button"
          variant="destructive"
        >
          <Trash2 className="size-3.5" />
          <span>Delete account</span>
        </Button>
      </SectionRow>
    </SectionRows>
  )
}

export const SESSION_ROW_CLASS = 'flex flex-col gap-3 py-3 sm:flex-row sm:items-center sm:justify-between'

export function CliSessionListSkeleton() {
  return (
    <ul className="divide-y divide-border border-y border-border">
      {(['medium', 'long'] as const).map((length) => (
        <li className={SESSION_ROW_CLASS} key={length}>
          <div className="min-w-0">
            <TextSkeleton length={length} />
            <TextSkeleton className="mt-1" length="long" size="meta" />
          </div>
          <BlockSkeleton className="size-8 shrink-0" />
        </li>
      ))}
    </ul>
  )
}
