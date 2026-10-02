import type { GitHubSetupCheckResponse } from '../../api/types.generated'

/** The trigger each workflow needs to run on Scope's branches. */
export const GITHUB_TRIGGER_SNIPPET = `on:
  push:
    branches: ['scope/**']`

export type GitHubSetupCheckView = {
  running: boolean
  status: string
  /** GitHub's answer to a refused push, or why no workflow started. */
  problem: string | null
  candidates: { name: string; required: boolean }[]
}

/** What the settings page says about the latest connection test, if any. */
export function githubSetupCheckView(
  check: GitHubSetupCheckResponse | null,
  requiredChecks: string[],
): GitHubSetupCheckView | null {
  if (!check) return null
  const commit = check.commit_oid.slice(0, 7)
  const candidates = check.check_names.map((name) => ({
    name,
    required: requiredChecks.includes(name),
  }))
  const view = { candidates, problem: check.message, running: false }
  switch (check.state) {
    case 'pushing':
      return { ...view, running: true, status: `Sending main (${commit}) to ${check.branch}.` }
    case 'waiting':
      return {
        ...view,
        running: true,
        status: `Sent main (${commit}) to ${check.branch}. Waiting for workflows to finish.`,
      }
    case 'finished':
      return {
        ...view,
        status: check.message
          ? `The test of main (${commit}) finished.`
          : candidates.length
            ? `Workflows ran on main (${commit}). Choose the checks a request must pass.`
            : `Workflows ran on main (${commit}).`,
      }
    case 'failed':
      return { ...view, status: `The test could not send main (${commit}) to GitHub.` }
  }
}
