import type { RequestQueueItemResponse } from '../../api/types.generated'

const REQUEST_STACK_MAX_INDENT = 4

type RequestStackGuide = { column: number; dashed: boolean }

export type RequestStackPlacement = {
  depth: number
  guides: RequestStackGuide[]
  elbow: 'parent' | 'elsewhere' | 'chain' | null
  children: boolean
}

type StackRow = { item: RequestQueueItemResponse }

export type StackedRequestRow<T> = { row: T; placement: RequestStackPlacement; descendants: T[]; stack: number }

export type RequestStackMember<T> = { row: T; depth: number }

export type VisibleStackedRequestRow<T> = StackedRequestRow<T> & { folded: T[] }

type StackNode<T> = {
  row?: T
  first: number
  children: StackNode<T>[]
}

export function stackRequestRows<T extends StackRow>(rows: T[]): StackedRequestRow<T>[] {
  const nodes = new Map<string, StackNode<T>>()
  rows.forEach((row, index) => nodes.set(row.item.request.id, { row, first: index, children: [] }))
  const roots: StackNode<T>[] = []
  for (const row of rows) {
    const node = nodes.get(row.item.request.id)!
    const parent = row.item.request.stack_parent
    if (!parent) {
      roots.push(node)
      continue
    }
    const key = parent.kind === 'available' ? parent.request.id : `unavailable:${row.item.request.id}`
    let owner = nodes.get(key)
    if (!owner) {
      owner = { first: node.first, children: [] }
      nodes.set(key, owner)
      roots.push(owner)
    }
    owner.children.push(node)
  }
  const placed: StackedRequestRow<T>[] = []
  const seen = new Set<StackNode<T>>()
  let stacks = 0
  const visit = (node: StackNode<T>, stack: number, depth: number, guides: RequestStackGuide[], elbow: RequestStackPlacement['elbow'], last: boolean) => {
    if (seen.has(node)) return
    seen.add(node)
    const indented = depth <= REQUEST_STACK_MAX_INDENT
    const own = depth > 0 && indented && !last ? [...guides, { column: depth - 1, dashed: elbow === 'elsewhere' }] : guides
    const placement = {
      depth: Math.min(depth, REQUEST_STACK_MAX_INDENT),
      guides: own,
      elbow: indented ? elbow : 'chain',
      children: node.children.length > 0,
    } as const
    const entry = node.row && { row: node.row, placement, descendants: [] as T[], stack }
    if (entry) placed.push(entry)
    const start = placed.length
    node.children.forEach((child, index) =>
      visit(child, stack, depth + 1, own, node.row ? 'parent' : 'elsewhere', index === node.children.length - 1),
    )
    if (entry) entry.descendants = placed.slice(start).map(({ row }) => row)
  }
  sortByFirst(roots).forEach((root) => visit(root, stacks++, 0, [], null, true))
  for (const node of nodes.values()) {
    if (node.row && !seen.has(node)) visit(node, stacks++, 0, [], null, true)
  }
  return placed
}

export function foldStackedRequestRows<T extends StackRow>(
  rows: StackedRequestRow<T>[],
  foldedIds: ReadonlySet<string>,
): VisibleStackedRequestRow<T>[] {
  const visible: VisibleStackedRequestRow<T>[] = []
  for (let index = 0; index < rows.length; index++) {
    const entry = rows[index]
    const folded = foldedIds.has(entry.row.item.request.id) ? entry.descendants : []
    visible.push({ ...entry, folded })
    index += folded.length
  }
  return visible
}

export function requestStackMembers<T extends StackRow>(
  rows: StackedRequestRow<T>[],
  requestId: string,
): RequestStackMember<T>[] {
  const stack = rows.find(({ row }) => row.item.request.id === requestId)?.stack
  const members = rows.filter((entry) => entry.stack === stack)
  if (members.length < 2) return []
  const top = Math.min(...members.map(({ placement }) => placement.depth))
  return members.map(({ row, placement }) => ({ row, depth: placement.depth - top }))
}

function sortByFirst<T>(nodes: StackNode<T>[]): StackNode<T>[] {
  for (const node of nodes) {
    sortByFirst(node.children)
    node.first = Math.min(node.first, ...node.children.map((child) => child.first))
  }
  return nodes.sort((left, right) => left.first - right.first)
}
