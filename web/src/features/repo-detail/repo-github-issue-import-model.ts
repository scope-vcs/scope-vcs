import type { GitHubIssueImportResponse } from '../../api/types.generated'

const LISTED_SKIPPED_ISSUES = 20

type GitHubIssueImportView = {
  working: boolean
  failed: boolean
  status: string
  read: string
  skipped: Array<{ issue: string, reason: string }>
  unlistedSkipped: number
}

export function githubIssueImportView(
  issueImport: GitHubIssueImportResponse | null,
  viewName: string,
): GitHubIssueImportView | null {
  if (!issueImport) return null
  const error = issueImport.error?.trim().replace(/\.$/, '') ?? null
  const read = {
    read: readSoFar(issueImport),
    skipped: issueImport.skipped_issues
      .slice(0, LISTED_SKIPPED_ISSUES)
      .map(({ number, reason }) => ({ issue: `#${number}`, reason })),
    unlistedSkipped: Math.max(0, issueImport.skipped_issues.length - LISTED_SKIPPED_ISSUES),
  }
  switch (issueImport.state) {
    case 'queued':
    case 'running':
      if (error) {
        return {
          working: issueImport.state === 'running',
          failed: true,
          status: `Import failed: ${error}. Retrying.`,
          ...read,
        }
      }
      return {
        working: true,
        failed: false,
        status: issueImport.state === 'queued'
          ? `Waiting to import open issues into the ${viewName} view.`
          : `Importing open issues into the ${viewName} view.`,
        ...read,
      }
    case 'succeeded':
      return {
        working: false,
        failed: false,
        status: read.read
          ? `Import into the ${viewName} view finished.`
          : 'GitHub had no open issues to import.',
        ...read,
      }
    case 'failed':
      return {
        working: false,
        failed: true,
        status: `Import failed: ${error ?? 'GitHub did not answer'}.`,
        ...read,
      }
  }
}

function readSoFar(issueImport: GitHubIssueImportResponse) {
  const skipped = issueImport.skipped_issues.length
  return [
    issueImport.imported_count > 0 && `${issues(issueImport.imported_count)} imported.`,
    issueImport.already_imported_count > 0
      && `${issues(issueImport.already_imported_count)} already had a request.`,
    skipped > 0 && `${issues(skipped)} could not be imported:`,
  ].filter(Boolean).join(' ')
}

function issues(count: number) {
  return count === 1 ? '1 issue' : `${count} issues`
}
