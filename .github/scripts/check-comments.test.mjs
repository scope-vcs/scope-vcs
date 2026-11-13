import assert from 'node:assert/strict'
import test from 'node:test'
import { commentSyntax, findComments } from './check-comments.mjs'

const lines = (found) => found.map(({ line }) => line)

test('selects comment syntax by file type and skips exempt files', () => {
  assert.equal(commentSyntax('api/src/main.rs'), 'slash')
  assert.equal(commentSyntax('web/src/app.tsx'), 'slash')
  assert.equal(commentSyntax('.github/workflows/ci.yml'), 'hash')
  assert.equal(commentSyntax('web/.env.example'), 'hash')
  assert.equal(commentSyntax('deploy/Dockerfile.api'), 'hash')
  assert.equal(commentSyntax('dev/check', '#!/usr/bin/env bash'), 'hash')
  assert.equal(commentSyntax('dev/notes', 'plain text'), null)
  assert.equal(commentSyntax('docs/architecture.md'), null)
  assert.equal(commentSyntax('web/src/routeTree.gen.ts'), null)
  assert.equal(commentSyntax('web/src/api/types.generated.ts'), null)
  assert.equal(commentSyntax('crates/scope-postgres/src/migrations/m0046_drop.rs'), null)
  assert.equal(commentSyntax('crates/scope-postgres/src/migrations/mod.rs'), 'slash')
  assert.equal(commentSyntax('README.html'), 'html')
  assert.equal(commentSyntax('legal/upstream/notice.sh'), null)
})

test('reports line, block, and JSX comments in code', () => {
  const source = [
    'fn main() {',
    '    // explain the next line',
    '    let path = "https://example.com"; ',
    '    /* block */',
    '}',
  ].join('\n')
  assert.deepEqual(lines(findComments('src/main.rs', source)), [2, 4])
  assert.deepEqual(lines(findComments('src/view.tsx', '<div>\n  {/* note */}\n</div>')), [2])
  assert.deepEqual(lines(findComments('src/lib.rs', '/*tight*/\n/*! inner docs */')), [1, 2])
})

test('allows machine-read comments', () => {
  const source = [
    '/* global window */',
    '/// <reference types="vite/client" />',
  ].join('\n')
  assert.deepEqual(findComments('src/lib.ts', source), [])
})

test('reports lint and type-check suppressions outside generated files', () => {
  const source = [
    '// @ts-nocheck',
    '// @ts-expect-error',
    '// eslint-disable-next-line no-console',
    '/* oxlint-disable */',
    '/* eslint eqeqeq: "off" */',
  ].join('\n')
  assert.deepEqual(lines(findComments('src/view.ts', source)), [1, 2, 3, 4, 5])
  assert.deepEqual(findComments('src/routeTree.gen.ts', source), [])
})

test('reports safety notes and doc comments, including on API contract items', () => {
  const source = [
    '// SAFETY: the pointer is valid',
    'unsafe { call() }',
    '/// Shown in the generated contract.',
    '#[derive(Serialize, JsonSchema)]',
    'pub struct Response {',
    '    /// A field in the contract.',
    '    pub id: String,',
    '}',
  ].join('\n')
  assert.deepEqual(lines(findComments('crates/scope-api-contract/src/types.rs', source)), [1, 3, 6])
})

test('does not mistake shell globs or dereferences for comments', () => {
  const source = 'case "$candidate" in\n  /*)\n    echo "#not-a-comment"\n'
  assert.deepEqual(findComments('cli-downloads/src/installers.rs', source), [])
  assert.deepEqual(findComments('src/deref.rs', '    *clock += 1;\n'), [])
})

test('reports hash comments but allows shebangs and tool directives', () => {
  const source = [
    '#!/usr/bin/env bash',
    '# shellcheck disable=SC2086',
    '# explain the next step',
    'cat <<EOF',
    '#!/bin/sh',
    'EOF',
    'echo done # trailing labels are out of scope',
  ].join('\n')
  assert.deepEqual(lines(findComments('dev/script.sh', source)), [3])
})

test('reports HTML comments except legal notices', () => {
  const page = [
    '<!DOCTYPE html>',
    '<!-- Embeds a font (Copyright 2019 IBM Corp.) under the',
    '     SIL Open Font License 1.1. -->',
    '<!-- layout note -->',
    '<html></html>',
  ].join('\n')
  assert.deepEqual(lines(findComments('README.html', page)), [4])
})
