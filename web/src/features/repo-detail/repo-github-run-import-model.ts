import { GITHUB_RUN_IMPORT_MAX_COUNT, isRunImportCount } from '../../api/github-inputs'
import type { GitHubRunImportResponse } from '../../api/types.generated'

/** The count a maintainer typed, or `null` when it is not one a repository can import. */
export function parseRunImportCount(draft: string): number | null {
  const text = draft.trim()
  if (!/^\d+$/.test(text)) return null
  const count = Number(text)
  return isRunImportCount(count) ? count : null
}

export const RUN_IMPORT_COUNT_HINT = `Enter a whole number from 0 to ${GITHUB_RUN_IMPORT_MAX_COUNT}.`

export type GitHubRunImportView = {
  /** Still reading GitHub, or waiting to try again. */
  inProgress: boolean
  /** Waiting to try again after GitHub failed; importing now replaces it. */
  retrying: boolean
  status: string
  failed: boolean
}

/** What the settings page says about the latest import, if any. */
export function githubRunImportView(runImport: GitHubRunImportResponse | null): GitHubRunImportView | null {
  if (!runImport) return null
  const error = runImport.error?.trim().replace(/\.$/, '') ?? null
  switch (runImport.state) {
    case 'queued':
    case 'running':
      return error
        ? { inProgress: true, retrying: true, failed: true, status: `Import failed: ${error}. Retrying.` }
        : {
          inProgress: true,
          retrying: false,
          failed: false,
          status: `Importing up to ${runs(runImport.run_count)} from GitHub.`,
        }
    case 'succeeded':
      return {
        inProgress: false,
        retrying: false,
        failed: false,
        status: runImport.imported_count === 0
          ? 'GitHub had no runs to import.'
          : `Imported ${runs(runImport.imported_count)}.`,
      }
    case 'failed':
      return {
        inProgress: false,
        retrying: false,
        failed: true,
        status: `Import failed: ${error ?? 'GitHub did not answer'}.`,
      }
  }
}

function runs(count: number) {
  return count === 1 ? '1 run' : `${count} runs`
}
