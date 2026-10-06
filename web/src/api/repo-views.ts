import type { ViewDefinition, ViewId } from './types.generated'

const VIEW_ID = /^[a-z][a-z0-9_-]{0,31}$/

export type RepoViews = {
  definitions: readonly ViewDefinition[]
  full: ViewId | null
  anyone: ViewId | null
  get: (view: ViewId) => ViewDefinition | null
  name: (view: ViewId) => string
  mayRead: (reader: ViewId, target: ViewId) => boolean
  readableBy: (reader: ViewId) => ViewDefinition[]
  includedNames: (view: ViewId) => string[] | 'all'
}

export function parseViewId(value: unknown): ViewId {
  if (typeof value === 'string' && VIEW_ID.test(value)) return value
  throw new Error(`Unsupported view: ${String(value)}`)
}

export function repoViews(definitions: readonly ViewDefinition[]): RepoViews {
  const byId = new Map(definitions.map((definition) => [definition.id, definition]))
  const labelCache = new Map<ViewId, ReadonlySet<ViewId>>()

  function labels(view: ViewId): ReadonlySet<ViewId> {
    const cached = labelCache.get(view)
    if (cached) return cached
    const reached = new Set<ViewId>()
    const pending = byId.has(view) ? [view] : []
    while (pending.length > 0) {
      const id = pending.pop()!
      if (reached.has(id)) continue
      const definition = byId.get(id)
      if (!definition) continue
      reached.add(id)
      if (!Array.isArray(definition.includes)) {
        const everything = new Set(byId.keys())
        labelCache.set(view, everything)
        return everything
      }
      pending.push(...definition.includes)
    }
    labelCache.set(view, reached)
    return reached
  }

  const mayRead = (reader: ViewId, target: ViewId) => labels(reader).has(target)

  return {
    definitions,
    full: definitions.find((definition) => !Array.isArray(definition.includes))?.id ?? null,
    anyone: definitions.find((definition) => definition.readers === 'anyone')?.id ?? null,
    get: (view) => byId.get(view) ?? null,
    name: (view) => byId.get(view)?.name ?? view,
    mayRead,
    readableBy: (reader) => definitions.filter((definition) => mayRead(reader, definition.id)),
    includedNames: (view) => {
      const includes = byId.get(view)?.includes ?? []
      if (!Array.isArray(includes)) return 'all'
      return includes.map((id) => byId.get(id)?.name ?? id)
    },
  }
}
