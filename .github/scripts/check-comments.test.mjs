import assert from 'node:assert/strict'
import test from 'node:test'
import { commentSyntax, findComments } from './check-comments.mjs'

const lines = (found) => found.map(({ line }) => line)

test('selects comment syntax by file type and skips exempt files', () => {
  assert.equal(commentSyntax('api/src/main.rs'), 'slash')
  assert.equal(commentSyntax('web/src/app.tsx'), 'slash')
  assert.equal(commentSyntax('.github/workflows/ci.yml'), 'hash')
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
    '// SAFETY: the pointer is valid',
    '// for the duration of the call.',
    'unsafe { call() }',
    '// @ts-nocheck',
    '// eslint-disable-next-line no-console',
    '/* oxlint-disable */',
    '/* eslint eqeqeq: "off" */',
    '/* global window */',
  ].join('\n')
  assert.deepEqual(findComments('src/lib.rs', source), [])
})

test('a SAFETY continuation ends at the first non-comment line', () => {
  const source = '// SAFETY: valid\nunsafe { call() }\n// not part of SAFETY'
  assert.deepEqual(lines(findComments('src/lib.rs', source)), [3])
})

test('allows doc comments only on API contract items', () => {
  const contract = [
    '/// Shown in the generated contract.',
    '#[derive(',
    '    Serialize,',
    '    JsonSchema,',
    ')]',
    'pub struct Response {',
    '    /// A field in the contract {with braces}.',
    '    pub id: String,',
    '}',
    '',
    '/// Internal helper beside the contract.',
    'fn helper() {}',
  ].join('\n')
  assert.deepEqual(lines(findComments('api/src/http/responses.rs', contract)), [11])
  const importOnly = 'use schemars::JsonSchema;\n/// Internal helper.\nfn helper() {}'
  assert.deepEqual(lines(findComments('api/src/use_cases/run.rs', importOnly)), [2])
  assert.deepEqual(findComments('crates/scope-api-contract/src/git_oid.rs', '/// A Git object id.\npub struct GitOid(String);'), [])
})

test('does not mistake shell globs or dereferences for comments', () => {
  const source = 'case "$candidate" in\n  /*)\n    echo "#not-a-comment"\n'
  assert.deepEqual(findComments('cli/src/installers.rs', source), [])
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
