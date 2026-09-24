import { SectionRow } from '@/components/section-rows'
import { BlockSkeleton, TextSkeleton } from '@/components/ui/skeleton'
import { KeyRound, Monitor } from 'lucide-react'
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
