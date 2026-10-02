import type { RepoParams } from '../../api/types'

/**
 * What GitHub sent to /github/setup. The OAuth callback brings `code` and
 * `state`, or `error` when the person declined. The app's Setup URL brings
 * the person back after installing with installation details only, which
 * Scope ignores.
 */
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

/**
 * `pending` is the repository a maintainer was connecting when they left to
 * install the app. It names where to restart the OAuth step, nothing more:
 * GitHub's installation redirect carries no state Scope can verify.
 */
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

/**
 * Where to go once the repository is connected: the page connecting started
 * from, kept in session storage, when it is a page of that same repository.
 * Anything else, or nothing, leads to the repository's settings.
 */
export function githubReturnPath(stored: string | null, connected: RepoParams) {
  const base = `/${encodeURIComponent(connected.owner)}/${encodeURIComponent(connected.repo)}`
  if (stored === base) return stored
  const rest = stored?.startsWith(`${base}/`) ? stored.slice(base.length) : null
  const samePage = rest !== null
    && /^(\/[A-Za-z0-9._~%-]+)+$/.test(rest)
    && !rest.split('/').some((segment) => segment === '.' || segment === '..')
  return samePage ? `${base}${rest}` : `${base}/settings`
}
