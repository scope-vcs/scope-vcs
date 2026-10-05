import type { RepoParams } from '../../api/types'

export type GitHubSetupSearch = {
  code?: string
  error?: string
  state?: string
}

export type GitHubSetupStep =
  | { kind: 'callback'; code: string; state: string }
  | { kind: 'declined' }
  | { kind: 'resume'; target: RepoParams }
  | { kind: 'incomplete' }

export const PENDING_GITHUB_TARGET_KEY = 'scope.github-setup.pending-repository'

export function githubSetupStep(search: GitHubSetupSearch, pending: RepoParams | null): GitHubSetupStep {
  if (search.code && search.state) return { kind: 'callback', code: search.code, state: search.state }
  if (search.error) return { kind: 'declined' }
  if (pending) return { kind: 'resume', target: pending }
  return { kind: 'incomplete' }
}

export function encodePendingGitHubTarget(target: RepoParams) {
  return JSON.stringify({ owner: target.owner, repo: target.repo })
}

export function parsePendingGitHubTarget(value: string | null): RepoParams | null {
  if (!value) return null
  try {
    const target: unknown = JSON.parse(value)
    if (typeof target !== 'object' || target === null) return null
    const { owner, repo } = target as Partial<RepoParams>
    return isSegment(owner) && isSegment(repo) ? { owner, repo } : null
  } catch {
    return null
  }
}

function isSegment(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && !value.includes('/')
}

export const GITHUB_RETURN_PATH_KEY = 'scope.github-setup.return-path'

export function githubReturnPath(stored: string | null, connected: RepoParams) {
  const base = `/${encodeURIComponent(connected.owner)}/${encodeURIComponent(connected.repo)}`
  if (stored === base) return stored
  const rest = stored?.startsWith(`${base}/`) ? stored.slice(base.length) : null
  const samePage = rest !== null
    && /^(\/[A-Za-z0-9._~%-]+)+$/.test(rest)
    && !rest.split('/').some((segment) => segment === '.' || segment === '..')
  return samePage ? `${base}${rest}` : `${base}/settings`
}
