import type { RepositoryAccessResponse, ViewDefinition, ViewId } from './types.generated'

export const builtinViews: ViewDefinition[] = [
  { id: 'public', name: 'Public', includes: [], readers: 'anyone' },
  { id: 'private', name: 'Private', includes: 'all', readers: 'members' },
]

export function viewName(view: ViewId, views: readonly ViewDefinition[] = builtinViews) {
  return views.find((definition) => definition.id === view)?.name ?? view
}

export function fullView(views: readonly ViewDefinition[] = builtinViews) {
  return views.find((definition) => definition.includes === 'all')?.id
}

export function anyoneView(views: readonly ViewDefinition[] = builtinViews) {
  return views.find((definition) => definition.readers === 'anyone')?.id
}

export function mayReadView(
  access: RepositoryAccessResponse,
  target: ViewId,
  views: readonly ViewDefinition[] = builtinViews,
) {
  if (!views.some((definition) => definition.id === target)) return false
  const pending = [access.view]
  const visited = new Set<ViewId>()
  while (pending.length > 0) {
    const id = pending.pop()!
    if (id === target) return true
    if (visited.has(id)) continue
    visited.add(id)
    const definition = views.find((candidate) => candidate.id === id)
    if (!definition) continue
    if (definition.includes === 'all') return true
    pending.push(...definition.includes)
  }
  return false
}

export function readableViews(
  access: RepositoryAccessResponse,
  views: readonly ViewDefinition[] = builtinViews,
) {
  return views.filter((definition) => mayReadView(access, definition.id, views))
}
