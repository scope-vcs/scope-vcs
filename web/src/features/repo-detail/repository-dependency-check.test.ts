import assert from 'node:assert/strict'
import test from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { RepositoryDependencyCheckView } from './repository-dependency-check-view'
import type { DependencyCheckPresentation } from './repository-dependency-model'

const presentation: DependencyCheckPresentation = {
  coverage: 'Checked main at a71c9f2 · 2 JS/TS files analyzed.',
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
    gaps: [{ path: '.', reason: 'analyzer output exceeded its limit' }],
    public_file_count: 1,
    unsupported_files: [],
  },
}

test('starts collapsed with navigable findings and a non-navigable repository gap', () => {
  const html = renderToStaticMarkup(createElement(RepositoryDependencyCheckView, {
    onSelectFilePath: () => undefined,
    presentation,
  }))

  assert.match(html, /^<details(?![^>]* open)/)
  assert.match(html, /<button[^>]*type="button"[^>]*>.*Public file.*src\/app\.ts.*<\/button>/)
  assert.match(html, /<button[^>]*type="button"[^>]*>.*Private file.*internal\/auth\.ts.*<\/button>/)
  assert.match(html, /Repository.*analyzer output exceeded its limit/)
  assert.equal((html.match(/<button /g) ?? []).length, 2)
})
