import { Button } from '@/components/ui/button'
import { isTypingTarget } from '@/lib/keyboard-target'
import { ChevronDown, ChevronUp } from 'lucide-react'
import { useEffect, useEffectEvent, useMemo, useState } from 'react'
import type { HistoryEntryDetailResponse } from '@/api/types.generated'
import { ChangedFilesWorkbench, type ChangedFilesProps, type useChangedFileNavigation } from './changed-files-workbench'
import { ChangedPathContext } from './changed-path-context'
import {
  changedPathFilterCounts,
  changedPaths,
  changedPathTargets,
  filterChangedPaths,
  findChangedPath,
  groupChangedPaths,
  initiallyOpenGroups,
  openableChangedPaths,
  steppedChangedPath,
  visibilitySummary,
  type ChangedPath,
  type ChangedPathFilter,
  type HistoryVisibilityChange,
} from './changed-paths-model'
import { ChangedPathsList, ChangedPathsToolbar } from './changed-paths-navigator'

export function HistoryEntryChanges({
  detail,
  navigation,
  onSelectVisibility,
  selectedVisibilityId,
  ...props
}: Omit<ChangedFilesProps, 'onCloseDiff'> & {
  detail: HistoryEntryDetailResponse
  navigation: ReturnType<typeof useChangedFileNavigation>
  onSelectVisibility: (change: HistoryVisibilityChange) => void
  selectedVisibilityId: string | null
}) {
  const { onSelectFile, selectedFilePath } = props
  const paths = useMemo(
    () => changedPaths(detail.files, detail.visibility_changes),
    [detail.files, detail.visibility_changes],
  )
  const counts = changedPathFilterCounts(paths)
  const [chosenFilter, setFilter] = useState<ChangedPathFilter>('all')
  const filter = counts.some((count) => count.filter === chosenFilter) ? chosenFilter : 'all'
  const [query, setQuery] = useState('')
  const [openGroups, setOpenGroups] = useState(() => initiallyOpenGroups(paths, selectedFilePath))
  const shown = filterChangedPaths(paths, filter, query)
  const shownGroups = groupChangedPaths(shown).groups.map((group) => group.name)
  const allOpen = shownGroups.every((group) => openGroups.has(group))
  const selected = selectedFilePath === null ? undefined : findChangedPath(paths, selectedFilePath)
  const openable = openableChangedPaths(shown)
  const position = selected ? openable.indexOf(selected) : -1

  function select(path: ChangedPath) {
    const [target] = changedPathTargets(path)
    if (!target) return
    if (target.kind === 'file') onSelectFile(target.file.path)
    else onSelectVisibility(target.change)
    if (path.group) setOpenGroups((open) => new Set(open).add(path.group))
    navigation.setOpen(false)
  }

  function step(direction: 1 | -1) {
    const next = steppedChangedPath(openable, position, direction)
    if (next && next !== selected) select(next)
  }
  const stepFromKeyboard = useEffectEvent(step)

  useEffect(() => {
    function handle(event: KeyboardEvent) {
      if (event.key !== 'j' && event.key !== 'k') return
      if (event.defaultPrevented || event.isComposing) return
      if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return
      if (isTypingTarget(event.target)) return
      if (document.querySelector(':popover-open, [role="dialog"], [role="alertdialog"]')) return
      event.preventDefault()
      stepFromKeyboard(event.key === 'j' ? 1 : -1)
    }
    document.addEventListener('keydown', handle)
    return () => document.removeEventListener('keydown', handle)
  }, [])

  useEffect(() => {
    navigation.ref.current
      ?.querySelector('[aria-current="true"]')
      ?.scrollIntoView({ block: 'nearest' })
  }, [navigation.ref, selectedFilePath])

  return (
    <ChangedFilesWorkbench
      {...props}
      diffActions={openable.length > 1 && (
        <div className="flex items-center gap-1 text-xs text-muted-foreground">
          {position >= 0 && <span className="mr-1 tabular-nums">{position + 1} of {openable.length}</span>}
          <Button
            aria-label="Previous changed file"
            disabled={position === 0}
            onClick={() => step(-1)}
            size="icon-xs"
            title="Previous changed file (k)"
            type="button"
            variant="ghost"
          >
            <ChevronUp className="size-3.5" />
          </Button>
          <Button
            aria-label="Next changed file"
            disabled={position === openable.length - 1}
            onClick={() => step(1)}
            size="icon-xs"
            title="Next changed file (j)"
            type="button"
            variant="ghost"
          >
            <ChevronDown className="size-3.5" />
          </Button>
        </div>
      )}
      diffContext={selected && (
        <ChangedPathContext
          onSelectFile={onSelectFile}
          onSelectVisibility={onSelectVisibility}
          path={selected}
          selectedVisibilityId={selectedVisibilityId}
          view={detail.view}
        />
      )}
      navigation={navigation}
      navigationLabel="Update file navigator"
      navigator={paths.length > 0 && (
        <ChangedPathsList
          countVisibility={filter !== 'visibility'}
          onSelect={select}
          onToggleGroup={(group) => setOpenGroups((open) => {
            const next = new Set(open)
            if (!next.delete(group)) next.add(group)
            return next
          })}
          openGroups={openGroups}
          paths={shown}
          query={query}
          selectedPath={selected?.path ?? null}
          view={detail.view}
        />
      )}
      toolbar={paths.length > 0 && (
        <ChangedPathsToolbar
          allOpen={allOpen}
          counts={counts}
          filter={filter}
          onFilterChange={(next) => {
            setFilter(next)
            navigation.setOpen(true)
          }}
          onOpenNavigation={() => navigation.setOpen(true)}
          onQueryChange={(next) => {
            setQuery(next)
            navigation.setOpen(true)
          }}
          onToggleAll={() => setOpenGroups((open) => {
            const next = new Set(open)
            for (const group of shownGroups) {
              if (allOpen) next.delete(group)
              else next.add(group)
            }
            return next
          })}
          query={query}
          visibility={visibilitySummary(paths)}
        />
      )}
    />
  )
}
