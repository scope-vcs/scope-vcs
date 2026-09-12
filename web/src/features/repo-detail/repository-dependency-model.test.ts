import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepositoryDependencyCheckResponse } from '../../api/types.generated'
import { dependencyCheckPresentation } from './repository-dependency-model'

function response(
  overrides: Partial<RepositoryDependencyCheckResponse> = {},
): RepositoryDependencyCheckResponse {
  return {
    error: null,
    report: {
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
    },
    status: 'Ready',
    ...overrides,
  }
}

test('uses the distinct public file count while retaining every finding', () => {
  const presentation = dependencyCheckPresentation({
    refreshError: null,
    refreshing: false,
    response: response(),
  })

  assert.equal(presentation.kind, 'report')
  if (presentation.kind !== 'report') return
  assert.equal(presentation.label, '1 public file imports private files')
  assert.equal(presentation.report.findings.length, 2)
  assert.equal(presentation.meta, 'JS/TS only')
  assert.equal(
    presentation.coverage,
    'Checked main at a71c9f2 · 4 JS/TS files analyzed. 1 other source file was not checked.',
  )
})

test('does not call an incomplete zero-finding report clear', () => {
  const report = response().report!
  const presentation = dependencyCheckPresentation({
    refreshError: null,
    refreshing: false,
    response: response({
      report: {
        ...report,
        findings: [],
        gaps: [{ path: 'src/loader.ts', reason: 'variable import target' }],
        public_file_count: 0,
      },
    }),
  })

  assert.equal(presentation.kind, 'report')
  if (presentation.kind !== 'report') return
  assert.equal(presentation.label, 'Dependency check incomplete')
  assert.equal(presentation.meta, 'Check incomplete')
  assert.deepEqual(presentation.report.gaps, [
    { path: 'src/loader.ts', reason: 'variable import target' },
  ])
})

test('shows a quiet, scoped clear result only for a complete report', () => {
  const report = response().report!
  assert.deepEqual(dependencyCheckPresentation({
    refreshError: null,
    refreshing: false,
    response: response({ report: { ...report, findings: [], public_file_count: 0 } }),
  }), {
    kind: 'plain',
    label: 'No public → private imports found',
    meta: 'JS/TS only',
    tone: 'muted',
  })
})

test('retains findings and clear reports during updates and failures', () => {
  const report = response().report!
  const states = [
    { status: 'Ready', refreshing: true, refreshError: null, meta: 'Updating…' },
    { status: 'Updating', refreshing: false, refreshError: null, meta: 'Updating…' },
    { status: 'Ready', refreshing: false, refreshError: 'request failed', meta: 'Update failed' },
  ] as const

  for (const findings of [report.findings, []]) {
    for (const { status, refreshing, refreshError, meta } of states) {
      const retained = { ...report, findings, public_file_count: findings.length ? 1 : 0 }
      const presentation = dependencyCheckPresentation({
        refreshError,
        refreshing,
        response: response({ report: retained, status }),
      })

      assert.equal(presentation.kind, 'report')
      if (presentation.kind !== 'report') return
      assert.equal(presentation.report, retained)
      assert.equal(presentation.label, findings.length
        ? '1 public file imports private files'
        : 'No public → private imports found')
      assert.equal(presentation.meta, meta)
      assert.match(presentation.coverage, /^Showing main at a71c9f2/)
    }
  }
})

test('renders first-run, failed, and unsupported states without a report', () => {
  assert.equal(dependencyCheckPresentation({
    refreshError: null, refreshing: false, response: null,
  }).label, 'Checking JS/TS imports…')
  assert.equal(dependencyCheckPresentation({
    refreshError: null, refreshing: false, response: response({ report: null, status: 'Failed' }),
  }).label, 'Dependency check unavailable')
  assert.equal(dependencyCheckPresentation({
    refreshError: 'request failed', refreshing: false, response: null,
  }).label, 'Dependency check unavailable')
  assert.equal(dependencyCheckPresentation({
    refreshError: null, refreshing: false, response: response({ report: null, status: 'Unsupported' }),
  }).label, 'Dependency check does not support these source files yet')
})

test('unsupported status wins over a preserved zero-file report', () => {
  const report = response().report!
  assert.deepEqual(dependencyCheckPresentation({
    refreshError: null,
    refreshing: false,
    response: response({
      report: {
        ...report,
        analyzed_file_count: 0,
        findings: [],
        public_file_count: 0,
      },
      status: 'Unsupported',
    }),
  }), {
    kind: 'plain',
    label: 'Dependency check does not support these source files yet',
    meta: null,
    tone: 'muted',
  })
})
