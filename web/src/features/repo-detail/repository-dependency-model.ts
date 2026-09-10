import type { RepositoryDependencyCheckResponse } from '../../api/types.generated'

type DependencyReport = NonNullable<RepositoryDependencyCheckResponse['report']>

export type DependencyCheckPresentation =
  | {
      kind: 'plain'
      label: string
      meta: string | null
      tone: 'muted' | 'warning'
    }
  | {
      coverage: string
      gaps: DependencyReport['gaps']
      kind: 'report'
      label: string
      meta: string
      report: DependencyReport
      staleReason: string | null
    }

export function dependencyCheckPresentation({
  response,
  refreshError,
  refreshing,
}: {
  response: RepositoryDependencyCheckResponse | null
  refreshError: string | null
  refreshing: boolean
}): DependencyCheckPresentation {
  if (response?.status === 'Unsupported') {
    return {
      kind: 'plain',
      label: 'Dependency check does not support these source files yet',
      meta: null,
      tone: 'muted',
    }
  }
  if (!response?.report) return reportlessPresentation(response, refreshError)
  const report = response.report

  const staleReason = staleReportReason(response, refreshError, refreshing)
  const incomplete = report.gaps.length > 0
  if (report.findings.length === 0 && !incomplete && staleReason === null) {
    return {
      kind: 'plain',
      label: 'No public → private imports found',
      meta: 'JS/TS only',
      tone: 'muted',
    }
  }

  return {
    coverage: dependencyCoverage(report, staleReason !== null),
    gaps: report.gaps,
    kind: 'report',
    label: report.findings.length > 0
      ? `${report.public_file_count} public ${report.public_file_count === 1 ? 'file imports' : 'files import'} private files`
      : incomplete ? 'Dependency check incomplete' : 'No public → private imports found',
    meta: staleReason ?? (incomplete ? 'Check incomplete' : 'JS/TS only'),
    report,
    staleReason,
  }
}

function reportlessPresentation(
  response: RepositoryDependencyCheckResponse | null,
  refreshError: string | null,
): DependencyCheckPresentation {
  if (refreshError || response?.status === 'Failed') {
    return {
      kind: 'plain',
      label: 'Dependency check unavailable',
      meta: null,
      tone: 'warning',
    }
  }
  return {
    kind: 'plain',
    label: 'Checking JS/TS imports…',
    meta: null,
    tone: 'muted',
  }
}

function staleReportReason(
  response: RepositoryDependencyCheckResponse,
  refreshError: string | null,
  refreshing: boolean,
) {
  if (refreshError || response.status === 'Failed') return 'Update failed'
  if (refreshing || response.status === 'Updating' || response.status === 'Pending') return 'Updating…'
  return null
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
