import { FileSystemTree } from '@/components/file-system-tree'
import { VisibilityBadge } from '@/components/visibility-badge'
import { displayRouteFilePath } from '@/lib/route-file'
import { cn } from '@/lib/utils'
import { NavigationSearch } from '@/components/navigation-search'
import { useEffect, useId, useRef, useState } from 'react'
import { matchingRepositoryFiles } from './repository-file-navigation'
import type { RepoFileResponse } from '@/api/types.generated'

export function RepositoryFileNavigator({
  files,
  onOpenNavigation,
  onSelectFile,
  selectedPath,
}: {
  files: RepoFileResponse[]
  onOpenNavigation: () => void
  onSelectFile: (path: string, pinned: boolean) => void
  selectedPath: string | null
}) {
  const [query, setQuery] = useState('')
  const [activeIndex, setActiveIndex] = useState(0)
  const resultRef = useRef<HTMLUListElement>(null)
  const listId = useId()
  const searching = query.trim().length > 0
  const matches = matchingRepositoryFiles(files, query)
  const active = Math.min(activeIndex, Math.max(0, matches.length - 1))

  useEffect(() => {
    resultRef.current?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' })
  }, [active, query])

  function openFile(path: string) {
    setQuery('')
    setActiveIndex(0)
    onSelectFile(path, false)
  }

  if (files.length === 0) {
    return <p className="px-2 py-4 text-xs text-muted-foreground">No files yet.</p>
  }

  return (
    <>
      <div className="mb-2 px-1">
        <NavigationSearch
          clearLabel="Clear file search"
          describedBy={`${listId}-instructions`}
          label="Find file"
          onChange={(value) => {
            setQuery(value)
            setActiveIndex(0)
          }}
          onKeyDown={(event) => {
            if (!searching || !matches.length) return
            if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
              event.preventDefault()
              const direction = event.key === 'ArrowDown' ? 1 : -1
              setActiveIndex((active + direction + matches.length) % matches.length)
            } else if (event.key === 'Enter') {
              event.preventDefault()
              openFile(matches[active].path)
            }
          }}
          onOpen={onOpenNavigation}
          placeholder="Find file…"
          value={query}
        />
        <p className="sr-only" id={`${listId}-instructions`}>
          Find a filename or path. Use up and down arrows to choose a result, then Enter to open it.
        </p>
      </div>
      {searching && (
        <div>
          <output className="sr-only">
            {matches.length
              ? `${matches.length} matching files. ${displayRouteFilePath(matches[active].path)} selected.`
              : ''}
          </output>
          <ul aria-label="Matching files" className="max-h-[60dvh] overflow-y-auto" ref={resultRef}>
            {matches.map((file, index) => (
              <li key={file.path}>
                <button
                  className={cn(
                    'flex w-full min-w-0 items-center gap-2 rounded px-2 py-2 text-left text-xs hover:bg-muted',
                    index === active && 'bg-muted text-foreground',
                  )}
                  data-active={index === active}
                  onClick={() => openFile(file.path)}
                  onMouseMove={() => setActiveIndex(index)}
                  type="button"
                >
                  <span className="min-w-0 flex-1 break-all font-mono">
                    {displayRouteFilePath(file.path)}
                  </span>
                  <VisibilityBadge compact visibility={file.visibility} />
                </button>
              </li>
            ))}
          </ul>
          {!matches.length && (
            <output className="block px-2 py-4 text-xs text-muted-foreground">
              No matching visible files.
            </output>
          )}
        </div>
      )}
      <div hidden={searching}>
        <FileSystemTree
          compactVisibility
          files={files}
          metaColumnLabel={null}
          onActivateFile={(file) => onSelectFile(file.path, true)}
          onSelectFile={(file) => onSelectFile(file.path, false)}
          selectedFilePath={selectedPath}
        />
      </div>
    </>
  )
}
