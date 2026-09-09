import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'
import ts from 'typescript'
import { parseRepoParams } from './repo-params'
import * as parsers from './request-inputs'

const request = { owner: 'scope', repo: 'vcs', request_id: 'req_1' }
const discussion = { ...request, discussion_id: 'discussion_1' }

test('request inputs reject non-objects and missing required identifiers', () => {
  for (const input of [null, undefined, [], 'request', 1, {}, { ...request, request_id: ' ' }, { ...request, request_id: 1 }]) {
    assert.throws(() => parsers.parseRequestParams(input))
  }
  assert.deepEqual(parsers.parseRequestParams({ ...request, extra: 'discard' }), request)
  assert.throws(() => parsers.parseDiscussionActionInput(request))
  assert.throws(() => parsers.parseLoadRequestRevisionDiffInput({ ...request, path: '/a' }))
})

test('files preserve significant whitespace and reject invalid path values', () => {
  assert.equal(parsers.parseRepoFileInput({ ...request, path: '/ file ' }).path, '/ file ')
  for (const path of ['', ' ', null, 10, '/file\0name', 'x'.repeat(4097)]) {
    assert.throws(() => parsers.parseRepoFileInput({ ...request, path }))
  }
  assert.equal(parsers.parseLoadRequestRevisionDiffInput({ ...request, revision_id: 'rev', commit_oid: 'oid', path: '/a' }).path, '/a')
})

test('pagination and booleans reject coercion, overflow, and invalid bounds', () => {
  for (const limit of [0, -1, 101, 1.5, NaN, Infinity, '25', null]) {
    assert.throws(() => parsers.parseLoadDiscussionsInput({ ...request, limit }))
  }
  for (const after of [-1, 0.1, Infinity, Number.MAX_SAFE_INTEGER + 1, '0', undefined]) {
    assert.throws(() => parsers.parseLoadDiscussionChangesInput({ ...request, after }))
  }
  assert.equal(parsers.parseLoadDiscussionChangesInput({ ...request, after: 0 }).after, 0)
  assert.equal(parsers.parseLoadDiscussionsInput({ ...request, limit: 100, include_revision_anchor: false }).limit, 100)
  assert.throws(() => parsers.parseLoadDiscussionsInput({ ...request, include_revision_anchor: 'false' }))
  assert.throws(() => parsers.parseLoadRepliesInput({ ...discussion, before: -1 }))
  assert.throws(() => parsers.parseMarkDiscussionReadInput({ ...discussion, through_position: '1' }))
  assert.throws(() => parsers.parseLoadDiscussionsInput({ ...request, cursor: {} }))
})

test('request actions validate the action and only require handles for invitee actions', () => {
  for (const action of ['close', 'leave', 'merge', 'submit']) {
    assert.deepEqual(parsers.parseRequestActionInput({ ...request, action }), { ...request, action })
  }
  for (const action of ['add_invitee', 'remove_invitee']) {
    assert.throws(() => parsers.parseRequestActionInput({ ...request, action }))
    assert.equal(parsers.parseRequestActionInput({ ...request, action, handle: 'adam' }).action, action)
  }
  assert.throws(() => parsers.parseRequestActionInput({ ...request, action: 'delete' }))
})

test('ratings enforce integer scores and UTF-8 reason limits', () => {
  for (const score of [0, 6, 1.5, '5', NaN]) assert.throws(() => parsers.parseRateRequestInput({ ...request, score, reason: 'Good' }))
  assert.equal(parsers.parseRateRequestInput({ ...request, score: 5, reason: 'é'.repeat(512) }).score, 5)
  for (const reason of ['', ' ', 'é'.repeat(513)]) assert.throws(() => parsers.parseRateRequestInput({ ...request, score: 5, reason }))
})

test('discussion and reply payloads validate identifiers, anchors, and body byte limits', () => {
  const create = { ...request, anchor: null, client_discussion_id: 'client', body_markdown: '  hello\n' }
  assert.deepEqual(parsers.parseCreateDiscussionInput(create), create)
  assert.equal(parsers.parseCreateDiscussionInput({ ...create, body_markdown: 'é'.repeat(32768) }).body_markdown.length, 32768)
  for (const patch of [{ body_markdown: 'é'.repeat(32769) }, { body_markdown: ' ' }, { anchor: {} }, { client_discussion_id: 'x'.repeat(129) }]) {
    assert.throws(() => parsers.parseCreateDiscussionInput({ ...create, ...patch }))
  }
  const anchored = { ...create, anchor: { revision_id: 'rev', commit_oid: null, path: null } }
  assert.deepEqual(parsers.parseCreateDiscussionInput(anchored), anchored)
  const reply = { ...discussion, body_markdown: 'reply', client_reply_id: 'client', reply_to_reply_id: null }
  assert.deepEqual(parsers.parseCreateReplyInput(reply), reply)
  assert.throws(() => parsers.parseCreateReplyInput({ ...reply, reply_to_reply_id: 1 }))
  assert.equal(parsers.parseUpdateDescriptionInput({ ...request, description_markdown: '', expected_description_markdown: 'old' }).description_markdown, '')
  assert.throws(() => parsers.parseUpdateDescriptionInput({ ...request, description_markdown: 'x'.repeat(256 * 1024 + 1), expected_description_markdown: 'old' }))
})

test('attachment inputs enforce generated transfer and media target shapes', () => {
  const prepare = {
    ...request,
    declared_media_type: 'image/png',
    filename: 'screen.png',
    operation_id: 'operation-one',
    sha256: 'a'.repeat(64),
    size_bytes: 42,
    target: { discussion_id: null, kind: 'Discussion' as const },
  }
  assert.deepEqual(parsers.parsePrepareAttachmentInput(prepare), prepare)
  for (const patch of [
    { size_bytes: -1 },
    { sha256: 42 },
    { target: { discussion_id: null, kind: 'Unknown' } },
  ]) {
    assert.throws(() => parsers.parsePrepareAttachmentInput({ ...prepare, ...patch }))
  }

  const finish = {
    ...request,
    attachment_id: 'attachment-one',
    parts: [{ part_number: 1, sha256: 'b'.repeat(64), size_bytes: 42 }],
    upload_id: 'upload-one',
  }
  assert.deepEqual(parsers.parseFinishAttachmentInput(finish), finish)
  assert.throws(() => parsers.parseFinishAttachmentInput({ ...finish, parts: [{ ...finish.parts[0], part_number: '1' }] }))
  assert.deepEqual(
    parsers.parseRetryAttachmentInput({ ...request, attachment_id: 'attachment-one', operation_id: 'retry-one' }),
    { ...request, attachment_id: 'attachment-one', operation_id: 'retry-one' },
  )
  assert.deepEqual(
    parsers.parseGrantAttachmentInput({ ...request, attachment_id: 'attachment-one', target: { kind: 'original' } }),
    { ...request, attachment_id: 'attachment-one', target: { kind: 'original' } },
  )
  assert.throws(() => parsers.parseGrantAttachmentInput({
    ...request,
    attachment_id: 'attachment-one',
    target: { kind: 'derivative' },
  }))
})

test('request and file server functions bind input validators that reject malformed identifiers', () => {
  const routes = [
    '$owner.$repo.requests.$requestId.tsx',
    '$owner.$repo.requests.$requestId.index.tsx',
    '$owner.$repo.requests.$requestId.changes.tsx',
    '$owner.$repo._code.index.tsx',
  ]
  for (const route of routes) {
    const source = readFileSync(resolve('src/routes', route), 'utf8')
    const file = ts.createSourceFile(route, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
    const serverFactories = new Set<string>()
    const validators = new Map<string, (input: unknown) => unknown>()
    const parserModules: Record<string, Record<string, (input: unknown) => unknown>> = {
      '@/api/request-inputs': parsers,
      '@/api/repos': { parseRepoParams },
      '@/api/repo-params': { parseRepoParams },
    }
    for (const statement of file.statements) {
      if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) continue
      const bindings = statement.importClause?.namedBindings
      if (!bindings || !ts.isNamedImports(bindings)) continue
      const module = statement.moduleSpecifier.text
      for (const binding of bindings.elements) {
        const imported = binding.propertyName?.text ?? binding.name.text
        if (module === '@tanstack/react-start' && imported === 'createServerFn') serverFactories.add(binding.name.text)
        const parser = parserModules[module]?.[imported]
        if (parser) validators.set(binding.name.text, parser)
      }
    }

    let serverFunctions = 0
    const inspect = (node: ts.Node) => {
      if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && serverFactories.has(node.expression.text)) {
        serverFunctions += 1
        const location = `${route}:${file.getLineAndCharacterOfPosition(node.getStart()).line + 1}`
        const boundValidators: ts.Expression[] = []
        let chain: ts.Node = node
        while (ts.isPropertyAccessExpression(chain.parent) && ts.isCallExpression(chain.parent.parent)) {
          const method = chain.parent
          const call = chain.parent.parent
          if (method.name.text === 'validator') {
            assert.equal(call.arguments.length, 1, `${location}: validator needs one parser`)
            boundValidators.push(call.arguments[0])
          }
          chain = call
        }
        assert.equal(boundValidators.length, 1, `${location}: server function needs one input validator`)
        const binding = boundValidators[0]
        const parser = ts.isIdentifier(binding) ? validators.get(binding.text) : undefined
        assert.ok(parser, `${location}: validator must reference an API input parser`)
        for (const input of [null, [], {}, { owner: 'scope', repo: 42 }]) {
          assert.throws(() => parser(input), `${location}: validator accepted ${JSON.stringify(input)}`)
        }
      }
      ts.forEachChild(node, inspect)
    }
    inspect(file)
    assert.ok(serverFunctions > 0, `${route}: no server functions found`)
  }
})
