import type { RepositoryDependencyCheckResponse } from '../../api/types.generated'

type DependencyReport = NonNullable<RepositoryDependencyCheckResponse['report']>

export type DependencyCheckDetails = {
  coverage: string
  report: DependencyReport
}

export type DependencyCheckPresentation =
  | { kind: 'hidden' }
  | { kind: 'status'; label: string; details: DependencyCheckDetails | null }
  | { kind: 'leak'; label: string; meta: string; details: DependencyCheckDetails }

const UNAVAILABLE = 'Dependency check unavailable'

export function dependencyCheckPresentation({
  response,
  refreshError,
  refreshing,
}: {
  response: RepositoryDependencyCheckResponse | null
  refreshError: string | null
  refreshing: boolean
}): DependencyCheckPresentation {
  if (response?.status === 'Unsupported') return { kind: 'hidden' }
  const failed = refreshError !== null || response?.status === 'Failed'
  if (!response?.report) {
    return failed ? { kind: 'status', label: UNAVAILABLE, details: null } : { kind: 'hidden' }
  }
  const report = response.report
  const updating = refreshing || response.status === 'Updating' || response.status === 'Pending'
  const details = { coverage: dependencyCoverage(report, failed || updating), report }

  if (report.findings.length > 0) {
    return {
      details,
      kind: 'leak',
      label: `${report.public_file_count} public ${report.public_file_count === 1 ? 'file imports' : 'files import'} private files`,
      meta: failed ? 'Update failed' : updating ? 'Updating…' : 'JS/TS only',
    }
  }
  if (report.gaps.length > 0) return { details, kind: 'status', label: 'Dependency check incomplete' }
  if (failed) return { details, kind: 'status', label: UNAVAILABLE }
  return { kind: 'hidden' }
}

function dependencyCoverage(report: DependencyReport, stale: boolean) {
  const snapshot = stale ? 'Showing' : 'Checked'
  const commit = report.commit_oid.slice(0, 7)
  const analyzed = `${report.analyzed_file_count} JS/TS ${report.analyzed_file_count === 1 ? 'file' : 'files'} analyzed.`
  const unsupported = report.unsupported_files.length > 0
    ? ` ${report.unsupported_files.length} other source ${report.unsupported_files.length === 1 ? 'file was' : 'files were'} not checked.`
    : ''
  const staleNote = stale ? ' These results may have changed.' : ''
  return `${snapshot} main at ${commit} · ${analyzed}${unsupported}${staleNote}`
}
