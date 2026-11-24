import type { GitHubIssueImportResponse } from '../../api/types.generated'

const LISTED_SKIPPED_ISSUES = 20

type GitHubIssueImportView = {
  working: boolean
  failed: boolean
  status: string
  read: string
}

export function githubIssueImportView(
  issueImport: GitHubIssueImportResponse | null,
  viewName: string,
): GitHubIssueImportView | null {
  if (!issueImport) return null
  const error = issueImport.error?.trim().replace(/\.$/, '') ?? null
  const read = readSoFar(issueImport)
  switch (issueImport.state) {
    case 'queued':
    case 'running':
      if (error) {
        return {
          working: issueImport.state === 'running',
          failed: true,
          status: `Import failed: ${error}. Retrying.`,
          read,
        }
      }
      return {
        working: true,
        failed: false,
        status: issueImport.state === 'queued'
          ? `Waiting to import open issues into the ${viewName} view.`
          : `Importing open issues into the ${viewName} view.`,
        read,
      }
    case 'succeeded':
      return {
        working: false,
        failed: false,
        status: read
          ? `Import into the ${viewName} view finished.`
          : 'GitHub had no open issues to import.',
        read,
      }
    case 'failed':
      return {
        working: false,
        failed: true,
        status: `Import failed: ${error ?? 'GitHub did not answer'}.`,
        read,
      }
  }
}

function readSoFar(issueImport: GitHubIssueImportResponse) {
  const skipped = issueImport.skipped_issue_numbers
  const listed = skipped.slice(0, LISTED_SKIPPED_ISSUES).map((number) => `#${number}`).join(', ')
  const unlisted = skipped.length - LISTED_SKIPPED_ISSUES
  return [
    issueImport.imported_count > 0 && `${issues(issueImport.imported_count)} imported.`,
    issueImport.already_imported_count > 0
      && `${issues(issueImport.already_imported_count)} already had a request.`,
    skipped.length > 0
      && `${issues(skipped.length)} could not be imported: ${listed}${unlisted > 0 ? ` and ${unlisted} more` : ''}.`,
  ].filter(Boolean).join(' ')
}

function issues(count: number) {
  return count === 1 ? '1 issue' : `${count} issues`
}
