import type { RepoFile } from '@/api/types'
import { ChevronDown } from 'lucide-react'
import { useRef } from 'react'
import { repositoryResources } from './repository-file-navigation'

export function RepositoryResourcesMenu({
  files,
  onSelectFilePath,
}: {
  files: RepoFile[]
  onSelectFilePath: (path: string) => void
}) {
  const resources = repositoryResources(files)
  const detailsRef = useRef<HTMLDetailsElement>(null)
  if (!resources.length) return null

  function closeMenu() {
    if (!detailsRef.current) return
    detailsRef.current.open = false
    detailsRef.current.querySelector('summary')?.focus()
  }

  function closeOnBlur(target: EventTarget | null) {
    if (detailsRef.current && !detailsRef.current.contains(target as Node | null)) {
      detailsRef.current.open = false
    }
  }

  return (
    <details className="relative" ref={detailsRef}>
      <summary
        className="flex cursor-pointer list-none items-center gap-1.5 rounded px-2 py-1.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring [&::-webkit-details-marker]:hidden"
        onBlur={(event) => closeOnBlur(event.relatedTarget)}
        onKeyDown={(event) => {
          if (event.key === 'Escape') closeMenu()
        }}
      >
        Resources <ChevronDown aria-hidden="true" className="size-3.5" />
      </summary>
      <div className="absolute left-1/2 top-full z-50 mt-1 w-32 -translate-x-1/2 rounded border border-border bg-popover p-1 text-popover-foreground shadow-[var(--shadow-pop)]">
        {resources.map(({ label, path }) => (
          <button
            className="block w-full rounded px-3 py-2 text-left text-xs hover:bg-muted focus-visible:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
            key={label}
            onBlur={(event) => closeOnBlur(event.relatedTarget)}
            onKeyDown={(event) => {
              if (event.key === 'Escape') closeMenu()
            }}
            onClick={() => {
              onSelectFilePath(path)
              closeMenu()
            }}
            type="button"
          >
            {label}
          </button>
        ))}
      </div>
    </details>
  )
}
