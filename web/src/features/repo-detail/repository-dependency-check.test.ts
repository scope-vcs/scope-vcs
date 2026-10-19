import assert from 'node:assert/strict'
import test from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { DependencyCheckDetailsBody } from './repository-dependency-check-view'
import { repoViews } from '../../api/repo-views'
import type { DependencyCheckDetails } from './repository-dependency-model'

const details: DependencyCheckDetails = {
  coverage: 'Checked main at a71c9f2 · 2 JS/TS files analyzed.',
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

test('makes findings navigable and leaves the repository gap non-navigable', () => {
  const html = renderToStaticMarkup(createElement(DependencyCheckDetailsBody, {
    details,
    onSelectFilePath: () => undefined,
    views: repoViews([
      { id: 'public', name: 'Public', includes: [], readers: 'anyone' },
      { id: 'private', name: 'Private', includes: 'all', readers: 'assigned' },
    ]),
  }))

  assert.match(html, /<button[^>]*type="button"[^>]*>.*Public file.*src\/app\.ts.*<\/button>/)
  assert.match(html, /<button[^>]*type="button"[^>]*>.*Private file.*internal\/auth\.ts.*<\/button>/)
  assert.match(html, /Repository.*analyzer output exceeded its limit/)
  assert.equal((html.match(/<button /g) ?? []).length, 2)
})
