import assert from 'node:assert/strict'
import test from 'node:test'
import {
  assertRequestMermaidSource,
  REQUEST_MERMAID_MAX_SOURCE_LENGTH,
} from './request-mermaid-policy'

test('accepts ordinary Mermaid diagrams up to the source limit', () => {
  assert.doesNotThrow(() => assertRequestMermaidSource('flowchart LR\n  A --> B'))
  assert.doesNotThrow(() => assertRequestMermaidSource('A'.repeat(REQUEST_MERMAID_MAX_SOURCE_LENGTH)))
})

test('rejects empty and oversized Mermaid source', () => {
  assert.throws(() => assertRequestMermaidSource('  \n'), /cannot be empty/u)
  assert.throws(
    () => assertRequestMermaidSource('A'.repeat(REQUEST_MERMAID_MAX_SOURCE_LENGTH + 1)),
    /cannot exceed 20,000 characters/u,
  )
})

test('rejects author-controlled Mermaid configuration', () => {
  assert.throws(
    () => assertRequestMermaidSource('---\nconfig:\n  securityLevel: loose\n---\nflowchart LR\nA-->B'),
    /configuration frontmatter/u,
  )
  assert.throws(
    () => assertRequestMermaidSource("%%{init: {'securityLevel': 'loose'}}%%\nflowchart LR\nA-->B"),
    /directives/u,
  )
  assert.throws(
    () => assertRequestMermaidSource("%%{INIT: {'maxEdges': 999}}%%\nflowchart LR\nA-->B"),
    /directives/u,
  )
})

test('rejects source features that can refer to external resources', () => {
  const blocked = [
    'flowchart LR\n  click A href "https://example.com"',
    'flowchart LR; A --> B; click A call callback()',
    'flowchart LR\n  A@{ img: "/avatar.png" }',
    'flowchart LR\n  A@{ "img": "//example.invalid/avatar.png" }',
    'flowchart LR\n  A@{ "\\u0069mg": "/avatar.png" }',
    'flowchart LR\n  A@{ icon: "logos:github-icon" }',
    'flowchart LR\n  A[<img src=/avatar.png>]',
    'flowchart LR\n  classDef risky fill:url(//example.com/pixel)',
    'sequenceDiagram\n  link Alice: Profile @ /people/alice',
  ]
  for (const source of blocked) {
    assert.throws(() => assertRequestMermaidSource(source))
  }
})

test('rejects custom styles without mistaking ordinary labels for style statements', () => {
  assert.throws(
    () => assertRequestMermaidSource('flowchart LR; style A fill:#f1f5f9'),
    /Custom Mermaid styling/u,
  )
  assert.throws(
    () => assertRequestMermaidSource('flowchart LR\n  linkStyle 0 stroke:#64748b'),
    /Custom Mermaid styling/u,
  )
  assert.doesNotThrow(() => assertRequestMermaidSource('flowchart LR\n  A[Style guide] --> B'))
})
