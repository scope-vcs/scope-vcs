import type { GitHubConnectionResponse } from '@/api/types.generated'

export type RunsCiOffer =
  | { kind: 'none' }
  | { kind: 'connect'; canConnect: boolean }

export const RUNS_COME_FROM_GITHUB = 'Runs come from this project’s GitHub Actions workflows once GitHub is connected.'

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
