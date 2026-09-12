import { Popover } from '@/components/ui/popover'
import { ChevronDown } from 'lucide-react'
import { repositoryResources } from './repository-file-navigation'
import type { RepoFileResponse } from '@/api/types.generated'

export function RepositoryResourcesMenu({
  files,
  onSelectFilePath,
}: {
  files: RepoFileResponse[]
  onSelectFilePath: (path: string) => void
}) {
  const resources = repositoryResources(files)
  if (!resources.length) return null

  return (
    <Popover
      align="center"
      className="w-32 rounded p-1"
      label="Repository resources"
      panel={(close) => resources.map(({ label, path }) => (
        <button
          className="block w-full rounded px-3 py-2 text-left text-xs hover:bg-muted focus-visible:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
          key={label}
          onClick={() => {
            onSelectFilePath(path)
            close()
          }}
          type="button"
        >
          {label}
        </button>
      ))}
      trigger={(props) => (
        <button
          className="flex cursor-pointer items-center gap-1.5 rounded px-2 py-1.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
          type="button"
          {...props}
        >
          Resources <ChevronDown aria-hidden="true" className="size-3.5" />
        </button>
      )}
    />
  )
}
