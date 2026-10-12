import type { GitHubSetupCheckResponse } from '../../api/types.generated'

export const GITHUB_TRIGGER_SNIPPET = `on:
  push:
    branches: ['scope/**']`

export type GitHubSetupCheckView = {
  running: boolean
  status: string
  problem: string | null
}

export function githubSetupCheckView(
  check: GitHubSetupCheckResponse | null,
): GitHubSetupCheckView | null {
  if (!check) return null
  const commit = check.commit_oid.slice(0, 7)
  const view = { problem: check.message, running: false }
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
          : check.check_names.length
            ? `Workflows ran on main (${commit}). Select the results required before merge.`
            : `No results were found on main (${commit}). Check that workflows include the scope/** push trigger.`,
      }
    case 'failed':
      return { ...view, status: `The test could not send main (${commit}) to GitHub.` }
  }
}
