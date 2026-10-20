import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestQueueItemResponse, RequestStackParentResponse } from '../../api/types.generated'
import { foldStackedRequestRows, stackRequestRows } from './request-workspace-stack'

function row(id: string, parent: string | 'unavailable' | null = null) {
  const stack_parent: RequestStackParentResponse | null =
    parent === null
      ? null
      : parent === 'unavailable'
        ? { kind: 'unavailable' }
        : { kind: 'available', request: { id: parent, name: parent, title: parent, state: 'Open', stack_status: 'main', stack_depth: 1, head_oid: 'head', current_revision_id: 'revision' } }
  return { item: { request: { id, stack_parent } } as unknown as RequestQueueItemResponse }
}

function layout(rows: ReturnType<typeof row>[]) {
  return stackRequestRows(rows).map(({ row, placement }) => [row.item.request.id, placement] as const)
}

test('rows without stacks keep their attention order', () => {
  assert.deepEqual(layout([row('a'), row('b')]), [
    ['a', { depth: 0, guides: [], elbow: null, children: false }],
    ['b', { depth: 0, guides: [], elbow: null, children: false }],
  ])
})

test('a stack moves to its most urgent request and lists parents before children', () => {
  assert.deepEqual(layout([row('other'), row('child', 'parent'), row('later'), row('parent'), row('grandchild', 'child')]), [
    ['other', { depth: 0, guides: [], elbow: null, children: false }],
    ['parent', { depth: 0, guides: [], elbow: null, children: true }],
    ['child', { depth: 1, guides: [], elbow: 'parent', children: true }],
    ['grandchild', { depth: 2, guides: [], elbow: 'parent', children: false }],
    ['later', { depth: 0, guides: [], elbow: null, children: false }],
  ])
})

test('siblings branch from one parent and the line continues past the earlier sibling', () => {
  assert.deepEqual(layout([row('parent'), row('first', 'parent'), row('nested', 'first'), row('second', 'parent')]), [
    ['parent', { depth: 0, guides: [], elbow: null, children: true }],
    ['first', { depth: 1, guides: [{ column: 0, dashed: false }], elbow: 'parent', children: true }],
    ['nested', { depth: 2, guides: [{ column: 0, dashed: false }], elbow: 'parent', children: false }],
    ['second', { depth: 1, guides: [], elbow: 'parent', children: false }],
  ])
})

test('requests whose parent is elsewhere hang from a shared parent outside the list', () => {
  assert.deepEqual(layout([row('child', 'hidden'), row('plain'), row('sibling', 'hidden'), row('opaque', 'unavailable')]), [
    ['child', { depth: 1, guides: [{ column: 0, dashed: true }], elbow: 'elsewhere', children: false }],
    ['sibling', { depth: 1, guides: [], elbow: 'elsewhere', children: false }],
    ['plain', { depth: 0, guides: [], elbow: null, children: false }],
    ['opaque', { depth: 1, guides: [], elbow: 'elsewhere', children: false }],
  ])
})

test('indentation stops at the cap and deeper requests join their parent in a straight line', () => {
  const chain = ['r0', 'r1', 'r2', 'r3', 'r4', 'r5', 'r6'].map((id, index, ids) => row(id, index ? ids[index - 1] : null))
  assert.deepEqual(layout([...chain, row('r6b', 'r5')]).map(([id, { depth, guides, elbow }]) => [id, depth, guides.map(({ column }) => column), elbow]), [
    ['r0', 0, [], null],
    ['r1', 1, [], 'parent'],
    ['r2', 2, [], 'parent'],
    ['r3', 3, [], 'parent'],
    ['r4', 4, [], 'parent'],
    ['r5', 4, [], 'chain'],
    ['r6', 4, [], 'chain'],
    ['r6b', 4, [], 'chain'],
  ])
})

test('laying out an already stacked list keeps its order', () => {
  const rows = [row('child', 'parent'), row('a'), row('parent'), row('second', 'parent')]
  const ordered = stackRequestRows(rows).map(({ row }) => row)
  assert.deepEqual(stackRequestRows(ordered).map(({ row }) => row), ordered)
})

test('inconsistent parents still render every request once', () => {
  assert.deepEqual(stackRequestRows([row('a', 'b'), row('b', 'a'), row('c')]).map(({ row }) => row.item.request.id).sort(), ['a', 'b', 'c'])
})

test('folding a stack hides its requests and reports them on the parent', () => {
  const stacked = stackRequestRows([row('parent'), row('first', 'parent'), row('nested', 'first'), row('second', 'parent'), row('plain')])
  const visible = (folded: string[]) =>
    foldStackedRequestRows(stacked, new Set(folded)).map(({ row, folded }) => [row.item.request.id, folded.map(({ item }) => item.request.id)])
  assert.deepEqual(visible(['first']), [['parent', []], ['first', ['nested']], ['second', []], ['plain', []]])
  assert.deepEqual(visible(['parent', 'first']), [['parent', ['first', 'nested', 'second']], ['plain', []]])
  assert.deepEqual(visible(['nested', 'plain']), visible([]))
})
