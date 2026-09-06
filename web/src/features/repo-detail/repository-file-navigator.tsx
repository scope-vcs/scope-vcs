import type { RepoFile } from '@/api/types'
import { FileSystemTree } from '@/components/file-system-tree'
import { VisibilityBadge } from '@/components/visibility-badge'
import { displayRouteFilePath } from '@/lib/route-file'
import { cn } from '@/lib/utils'
import { Search, X } from 'lucide-react'
import { useEffect, useId, useRef, useState } from 'react'
import { matchingRepositoryFiles } from './repository-file-navigation'

export function RepositoryFileNavigator({
  files,
  onOpenNavigation,
  onSelectFile,
  selectedPath,
}: {
  files: RepoFile[]
  onOpenNavigation: () => void
  onSelectFile: (path: string, pinned: boolean) => void
  selectedPath: string | null
}) {
  const [query, setQuery] = useState('')
  const [activeIndex, setActiveIndex] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)
  const resultRef = useRef<HTMLUListElement>(null)
  const listId = useId()
  const searching = query.trim().length > 0
  const matches = matchingRepositoryFiles(files, query)
  const active = Math.min(activeIndex, Math.max(0, matches.length - 1))

  useEffect(() => {
    function findFile(event: KeyboardEvent) {
      if (
        event.key !== '/' || event.defaultPrevented || event.isComposing ||
        event.metaKey || event.ctrlKey || event.altKey
      ) return
      const target = event.target
      if (
        target instanceof HTMLElement &&
        target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"]')
      ) return
      if (!inputRef.current) return
      event.preventDefault()
      onOpenNavigation()
      requestAnimationFrame(() => inputRef.current?.focus())
    }
    document.addEventListener('keydown', findFile)
    return () => document.removeEventListener('keydown', findFile)
  }, [onOpenNavigation])

  useEffect(() => {
    resultRef.current?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' })
  }, [active, query])

  function clearSearch() {
    setQuery('')
    setActiveIndex(0)
    inputRef.current?.focus()
  }

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
        <div className="relative flex items-center">
          <Search aria-hidden="true" className="pointer-events-none absolute left-2 size-3.5 text-muted-foreground" />
          <input
            aria-describedby={`${listId}-instructions`}
            aria-label="Find file"
            autoComplete="off"
            className="h-8 w-full min-w-0 rounded border border-border bg-background pr-8 pl-7 text-xs placeholder:text-muted-foreground focus-visible:outline-2 focus-visible:outline-ring"
            onChange={(event) => {
              setQuery(event.target.value)
              setActiveIndex(0)
            }}
            onKeyDown={(event) => {
              if (event.key === 'Escape') {
                event.preventDefault()
                clearSearch()
                return
              }
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
            placeholder="Find file…"
            ref={inputRef}
            type="search"
            value={query}
          />
          {query ? (
            <button
              aria-label="Clear file search"
              className="absolute right-1 rounded p-1 text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
              onClick={clearSearch}
              type="button"
            >
              <X aria-hidden="true" className="size-3.5" />
            </button>
          ) : <kbd aria-hidden="true" className="pointer-events-none absolute right-2 text-[11px] text-muted-foreground">/</kbd>}
        </div>
        <p className="sr-only" id={`${listId}-instructions`}>
          Find a filename or path. Use up and down arrows to choose a result, then Enter to open it.
        </p>
      </div>
      {searching && (
        <div>
          <output className="sr-only">
            {matches.length ? `${matches.length} matching files. ${displayRouteFilePath(matches[active].path)} selected.` : ''}
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
                  <span className="min-w-0 flex-1 break-all font-mono">{displayRouteFilePath(file.path)}</span>
                  <VisibilityBadge compact visibility={file.visibility} />
                </button>
              </li>
            ))}
          </ul>
          {!matches.length && (
            <output className="block px-2 py-4 text-xs text-muted-foreground">No matching visible files.</output>
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
