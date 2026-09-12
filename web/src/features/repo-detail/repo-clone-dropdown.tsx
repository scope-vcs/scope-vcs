import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { Button } from '@/components/ui/button'
import { Popover } from '@/components/ui/popover'
import { cn } from '@/lib/utils'
import { ChevronDown, Code2 } from 'lucide-react'
import {
  permissionedCloneCommand,
  publicCloneCommand,
} from './clone-command'
import type { RepoSummaryResponse } from '@/api/types.generated'

export function RepoCloneDropdown({
  cloneRemoteUrl,
  repo,
}: {
  cloneRemoteUrl: string
  repo: RepoSummaryResponse
}) {
  const permissioned = repo.access.actor !== 'Public'
  const cloneCommand = permissioned
    ? permissionedCloneCommand(repo.owner_handle, repo.name)
    : publicCloneCommand(cloneRemoteUrl)
  const cloneLabel = permissioned ? 'Scope CLI' : 'Public HTTPS'
  const copyLabel = permissioned
    ? 'Copy permissioned clone command'
    : 'Copy public clone command'

  return (
    <Popover
      className="mt-2 w-[min(420px,calc(100vw-2rem))] border-[var(--border-strong)]"
      label={`${cloneLabel} clone command`}
      panel={() => (
        <>
          <div className="mb-2 flex h-6 items-center justify-between text-xs font-semibold leading-4">
            <span>{cloneLabel}</span>
          </div>
          <CopyableCodeBlock copyLabel={copyLabel} value={cloneCommand} />
        </>
      )}
      trigger={(props) => (
        <Button size="sm" type="button" variant="secondary" {...props}>
          <Code2 className="size-3.5" />
          <span>Clone</span>
          <span className="-my-2 ml-1 flex h-8 items-center border-l border-border pl-2">
            <ChevronDown
              className={cn(
                'size-3.5 text-muted-foreground transition-transform',
                props['aria-expanded'] && 'rotate-180',
              )}
            />
          </span>
        </Button>
      )}
    />
  )
}
