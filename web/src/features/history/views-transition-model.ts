import type {
  HistoryVisibilityChangeResponse,
  ViewDefinition,
  ViewsTransition,
} from '../../api/types.generated'

export function viewsTransitionChanges({ before, after }: ViewsTransition): string[] {
  const previous = new Map(before.map((view) => [view.id, view]))
  const next = new Map(after.map((view) => [view.id, view]))
  const name = (id: string) => next.get(id)?.name ?? previous.get(id)?.name ?? id
  const names = (ids: readonly string[]) => ids.map(name).join(', ')
  const changes: string[] = []

  for (const view of after) {
    const old = previous.get(view.id)
    if (!old) {
      changes.push(addedView(view, names))
      continue
    }
    if (old.name !== view.name) changes.push(`Renamed ${old.name} to ${view.name}`)
    const oldIncludes = includedIds(old)
    const newIncludes = includedIds(view)
    if (oldIncludes !== 'all' && newIncludes !== 'all') {
      const gained = newIncludes.filter((id) => !oldIncludes.includes(id))
      const lost = oldIncludes.filter((id) => !newIncludes.includes(id))
      if (gained.length > 0) changes.push(`${view.name} now includes ${names(gained)}`)
      if (lost.length > 0) changes.push(`${view.name} no longer includes ${names(lost)}`)
    }
    if (old.readers !== view.readers) {
      changes.push(view.readers === 'anyone'
        ? `${view.name} is now readable by anyone`
        : `${view.name} is now readable only by assigned members`)
    }
  }
  for (const view of before) {
    if (!next.has(view.id)) changes.push(`Removed ${view.name}`)
  }
  return changes
}

export function transitionPathLabel(change: HistoryVisibilityChangeResponse, viewName: string) {
  if (change.old_label !== change.new_label) return null
  return change.file?.kind === 'Deleted' ? `Left ${viewName}` : `Entered ${viewName}`
}

function addedView(view: ViewDefinition, names: (ids: readonly string[]) => string) {
  const includes = includedIds(view)
  if (includes === 'all') return `Added ${view.name}, including every view`
  return includes.length > 0 ? `Added ${view.name}, including ${names(includes)}` : `Added ${view.name}`
}

function includedIds(view: ViewDefinition): readonly string[] | 'all' {
  return Array.isArray(view.includes) ? view.includes : 'all'
}
