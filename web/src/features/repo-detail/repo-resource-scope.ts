import type { RepoSummary } from '../../api/types'

export function repoResourceScope(repo: RepoSummary, viewerId: string | null) {
  return JSON.stringify([repo.id, viewerId, repo.access])
}
