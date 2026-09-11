import type { RepoSummaryResponse } from '../../api/types.generated'

export function repoResourceScope(repo: RepoSummaryResponse, viewerId: string | null) {
  return JSON.stringify([repo.id, viewerId, repo.access])
}

export function repoResourceScopeOwner(accessScope: string) {
  try {
    const value: unknown = JSON.parse(accessScope)
    if (!Array.isArray(value) || typeof value[0] !== 'string') return null
    const viewer = typeof value[1] === 'string' ? value[1] : 'anonymous'
    return JSON.stringify([value[0], viewer])
  } catch {
    return null
  }
}
