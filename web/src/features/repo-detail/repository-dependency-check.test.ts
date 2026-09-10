import assert from 'node:assert/strict'
import test from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { RepositoryDependencyCheckView } from './repository-dependency-check-view'
import type { DependencyCheckPresentation } from './repository-dependency-model'

const presentation: DependencyCheckPresentation = {
  coverage: 'Checked main at a71c9f2 · 2 JS/TS files analyzed.',
  gaps: [],
  kind: 'report',
  label: '1 public file imports private files',
  meta: 'JS/TS only',
  report: {
    analyzed_file_count: 2,
    analyzer_version: '1',
    commit_oid: 'a71c9f2',
    findings: [{
      source_path: 'src/app.ts',
      target_path: 'internal/auth.ts',
    }],
    gaps: [],
    public_file_count: 1,
    unsupported_files: [],
  },
  staleReason: null,
}

test('starts collapsed with both file paths as buttons inside the disclosure', () => {
  const html = renderToStaticMarkup(createElement(RepositoryDependencyCheckView, {
    onSelectFilePath: () => undefined,
    presentation,
  }))

  assert.match(html, /^<details /)
  assert.doesNotMatch(html, /^<details[^>]* open/)
  assert.match(html, /<summary[^>]*>.*1 public file imports private files.*JS\/TS only.*<\/summary>/)
  assert.match(html, /<button[^>]*type="button"[^>]*>.*Public file.*src\/app\.ts.*<\/button>/)
  assert.match(html, /<button[^>]*type="button"[^>]*>.*Private file.*internal\/auth\.ts.*<\/button>/)
})

test('renders a repository-wide gap as text instead of a file button', () => {
  const html = renderToStaticMarkup(createElement(RepositoryDependencyCheckView, {
    onSelectFilePath: () => undefined,
    presentation: {
      ...presentation,
      gaps: [{ path: '.', reason: 'analyzer output exceeded its limit' }],
      report: {
        ...presentation.report,
        gaps: [{ path: '.', reason: 'analyzer output exceeded its limit' }],
      },
    },
  }))

  assert.match(html, /Repository.*analyzer output exceeded its limit/)
  assert.doesNotMatch(html, />\.<\/button>/)
})
