import { SectionRow, SectionRows } from '@/components/section-rows'
import { Button } from '@/components/ui/button'
import { MailPlus, Trash2, Users } from 'lucide-react'
import type { ReactNode } from 'react'

// The fixed parts of each settings section. The loaded page and its pending
// state both render these, so their titles, copy and controls cannot drift.

export function RepositoryDetailsSection({ children }: { children: ReactNode }) {
  return (
    <SectionRows className="mt-0 border-b border-border">
      <SectionRow
        description="Help visitors understand the project. These details are public."
        title="Repository details"
      >
        {children}
      </SectionRow>
    </SectionRows>
  )
}

export const REPOSITORY_DETAIL_FIELDS = {
  description: 'Description',
  website_url: 'Website or documentation',
} as const

/** A control without `onDelete` is drawn disabled while the page loads. */
export function DangerZoneSection({ onDelete }: { onDelete?: () => void }) {
  return (
    <SectionRows>
      <SectionRow
        description="Permanently removes repo metadata and stored Git data from Scope."
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
          <span>Delete repository</span>
        </Button>
      </SectionRow>
    </SectionRows>
  )
}

/** The owner row and invite control come first; member and invite lists follow. */
export function AccessSection({
  canInvite,
  children,
  onInvite,
  ownerHandle,
}: {
  canInvite: boolean
  children?: ReactNode
  onInvite?: () => void
  ownerHandle: string
}) {
  return (
    <SectionRows>
      <SectionRow
        description={
          canInvite
            ? 'Members can read private files and take part in maintainer reviews. Only the owner manages membership.'
            : 'Members can be invited after the first Scope push is applied.'
        }
        icon={<Users className="size-4" />}
        title="Access"
      >
        <div className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-3 text-sm">
            <div className="min-w-0">
              <div className="truncate font-medium leading-5">@{ownerHandle}</div>
              <div className="leading-5 text-muted-foreground">Owner · Full access</div>
            </div>
            <Button disabled={!canInvite || !onInvite} onClick={onInvite} size="sm" type="button">
              <MailPlus className="size-3.5" />
              <span>Invite member</span>
            </Button>
          </div>
          {children}
        </div>
      </SectionRow>
    </SectionRows>
  )
}
