import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { Button } from '@/components/ui/button'
import { Popover } from '@/components/ui/popover'
import { cn } from '@/lib/utils'
import { ChevronDown, Code2 } from 'lucide-react'
import { cloneCommands } from './clone-command'
import type { RepoSummaryResponse, ViewId } from '@/api/types.generated'

export function RepoCloneDropdown({
  cloneRemoteUrl,
  repo,
  view,
  viewName,
}: {
  cloneRemoteUrl: string
  repo: RepoSummaryResponse
  view: ViewId
  viewName: string
}) {
  const commands = cloneCommands({
    actor: repo.access.actor,
    owner: repo.owner_handle,
    readerView: repo.access.view,
    remoteUrl: cloneRemoteUrl,
    repo: repo.name,
    view,
  })

  return (
    <Popover
      className="mt-2 w-[min(420px,calc(100vw-2rem))] border-[var(--border-strong)]"
      label={`Clone the ${viewName} view`}
      panel={() => (
        <div className="grid grid-cols-1 gap-3">
          <p className="truncate text-xs text-muted-foreground">Cloning the {viewName} view</p>
          {commands.map((command) => (
            <div className="min-w-0" key={command.label}>
              <div className="mb-2 flex h-6 items-center text-xs font-semibold leading-4">{command.label}</div>
              <CopyableCodeBlock copyLabel={command.copyLabel} value={command.value} />
            </div>
          ))}
        </div>
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
