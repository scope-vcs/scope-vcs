export type RepositoryVersion =
  | { kind: 'main' }
  | { kind: 'request'; id: string }
  | { kind: 'update'; id: string }

export type RepositoryVersionSearch = { request?: string; update?: string }

export function parseRepositoryVersionSearch(search: Record<string, unknown>): RepositoryVersionSearch {
  if (typeof search.request === 'string' && search.request) return { request: search.request }
  if (typeof search.update === 'string' && search.update) return { update: search.update }
  return {}
}

export function repositoryVersion(search: RepositoryVersionSearch): RepositoryVersion {
  if (search.request) return { kind: 'request', id: search.request }
  if (search.update) return { kind: 'update', id: search.update }
  return { kind: 'main' }
}

export function repositoryVersionSearch(version: RepositoryVersion): RepositoryVersionSearch {
  if (version.kind === 'request') return { request: version.id }
  if (version.kind === 'update') return { update: version.id }
  return {}
}

export function repositoryVersionKey(version: RepositoryVersion) {
  return version.kind === 'main' ? 'main' : `${version.kind}:${version.id}`
}
