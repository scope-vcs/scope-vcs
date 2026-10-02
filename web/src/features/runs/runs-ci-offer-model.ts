import type { GitHubConnectionResponse } from '@/api/types.generated'

/** What the Runs page offers when it has nothing to list. */
export type RunsCiOffer =
  | { kind: 'none' }
  | { kind: 'connect'; canConnect: boolean }

export const RUNS_COME_FROM_GITHUB = 'Runs come from this project’s GitHub Actions workflows once GitHub is connected.'

/**
 * A repository with no workflows of its own and no GitHub link gets its runs
 * from GitHub Actions once a maintainer connects it, so the empty page says so
 * and offers maintainers to connect. A server without GitHub promises nothing.
 * `configured` is what the Runs page read; `github` is the maintainer's
 * connection from the settings resource, `null` until it loads.
 */
export function runsCiOffer({
  configured,
  github,
  hasWorkflows,
  maintainer,
}: {
  configured: boolean
  github: GitHubConnectionResponse | null
  hasWorkflows: boolean
  maintainer: boolean
}): RunsCiOffer {
  if (hasWorkflows || !configured) return { kind: 'none' }
  if (!maintainer || !github) return { kind: 'connect', canConnect: false }
  if (!github.configured || github.connection) return { kind: 'none' }
  return { kind: 'connect', canConnect: true }
}
