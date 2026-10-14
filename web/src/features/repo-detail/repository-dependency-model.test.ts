import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepositoryDependencyCheckResponse } from '../../api/types.generated'
import { dependencyCheckPresentation } from './repository-dependency-model'

type Report = NonNullable<RepositoryDependencyCheckResponse['report']>

const leakingReport: Report = {
  analyzed_file_count: 4,
  analyzer_version: '1',
  commit_oid: 'a71c9f2e817d',
  findings: [
    { source_path: 'src/app.ts', target_path: 'internal/auth.ts' },
    { source_path: 'src/app.ts', target_path: 'internal/billing.ts' },
  ],
  gaps: [],
  public_file_count: 1,
  unsupported_files: ['src/main.rs'],
}
const clearReport: Report = { ...leakingReport, findings: [], public_file_count: 0 }
const incompleteReport: Report = {
  ...clearReport,
  gaps: [{ path: 'src/loader.ts', reason: 'variable import target' }],
}

function present(
  overrides: Partial<RepositoryDependencyCheckResponse> | null,
  { refreshError = null, refreshing = false }: { refreshError?: string | null; refreshing?: boolean } = {},
) {
  return dependencyCheckPresentation({
    refreshError,
    refreshing,
    response: overrides && { error: null, report: leakingReport, status: 'Ready', ...overrides },
  })
}

test('public files importing private files are a leak with every finding retained', () => {
  const presentation = present({})
  assert.equal(presentation.kind, 'leak')
  if (presentation.kind !== 'leak') return
  assert.equal(presentation.label, '1 public file imports private files')
  assert.equal(presentation.meta, 'JS/TS only')
  assert.equal(presentation.details.report.findings.length, 2)
  assert.equal(
    presentation.details.coverage,
    'Checked main at a71c9f2 · 4 JS/TS files analyzed. 1 other source file was not checked.',
  )
})

test('a leak stays visible while its report updates or fails to refresh', () => {
  const states = [
    { overrides: {}, refresh: { refreshing: true }, meta: 'Updating…' },
    { overrides: { status: 'Updating' as const }, refresh: {}, meta: 'Updating…' },
    { overrides: {}, refresh: { refreshError: 'request failed' }, meta: 'Update failed' },
  ]
  for (const { overrides, refresh, meta } of states) {
    const presentation = present(overrides, refresh)
    assert.equal(presentation.kind, 'leak')
    if (presentation.kind !== 'leak') return
    assert.equal(presentation.meta, meta)
    assert.match(presentation.details.coverage, /^Showing main at a71c9f2.*These results may have changed\.$/)
  }
})

test('an incomplete zero-finding report is a status with its gaps, never hidden as clear', () => {
  const presentation = present({ report: incompleteReport })
  assert.equal(presentation.kind, 'status')
  if (presentation.kind !== 'status') return
  assert.equal(presentation.label, 'Dependency check incomplete')
  assert.deepEqual(presentation.details?.report.gaps, incompleteReport.gaps)
})

test('a failed check is a status, keeping the last clear report when one exists', () => {
  assert.deepEqual(present({ report: null, status: 'Failed' }), {
    details: null,
    kind: 'status',
    label: 'Dependency check unavailable',
  })
  assert.deepEqual(present(null, { refreshError: 'request failed' }), {
    details: null,
    kind: 'status',
    label: 'Dependency check unavailable',
  })
  const stale = present({ report: clearReport }, { refreshError: 'request failed' })
  assert.equal(stale.kind, 'status')
  if (stale.kind !== 'status') return
  assert.equal(stale.label, 'Dependency check unavailable')
  assert.equal(stale.details?.report, clearReport)
})

test('clear, updating-clear, first-run, and unsupported checks stay hidden', () => {
  assert.deepEqual(present({ report: clearReport }), { kind: 'hidden' })
  assert.deepEqual(present({ report: clearReport }, { refreshing: true }), { kind: 'hidden' })
  assert.deepEqual(present(null), { kind: 'hidden' })
  assert.deepEqual(present({ report: null, status: 'Pending' }), { kind: 'hidden' })
  assert.deepEqual(present({ status: 'Unsupported' }), { kind: 'hidden' })
})
