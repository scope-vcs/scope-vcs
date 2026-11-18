import type { RepoSummaryResponse } from '../../api/types.generated'

export function repoResourceScope(repo: RepoSummaryResponse, viewerId: string | null) {
  return JSON.stringify([repo.id, viewerId, repo.access])
}

function scopeParts(accessScope: string): [string, unknown] | null {
  try {
    const value: unknown = JSON.parse(accessScope)
    return Array.isArray(value) && typeof value[0] === 'string' ? [value[0], value[1]] : null
  } catch {
    return null
  }
}

export function repoResourceScopeRepoId(accessScope: string) {
  return scopeParts(accessScope)?.[0] ?? null
}

export function repoResourceScopeOwner(accessScope: string) {
  const parts = scopeParts(accessScope)
  if (!parts) return null
  return JSON.stringify([parts[0], typeof parts[1] === 'string' ? parts[1] : 'anonymous'])
}
