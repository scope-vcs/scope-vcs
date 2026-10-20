import type { RequestQueueItemResponse } from '../../api/types.generated'

export type RequestStackPlacement = {
  depth: number
  guides: number[]
  elbow: 'parent' | 'elsewhere' | null
  children: boolean
}

type StackRow = { item: RequestQueueItemResponse }

type StackNode<T> = {
  row?: T
  first: number
  children: StackNode<T>[]
}

export function stackRequestRows<T extends StackRow>(rows: T[]): { row: T; placement: RequestStackPlacement }[] {
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
  const placed: { row: T; placement: RequestStackPlacement }[] = []
  const seen = new Set<StackNode<T>>()
  const visit = (node: StackNode<T>, depth: number, guides: number[], elbow: RequestStackPlacement['elbow'], last: boolean) => {
    if (seen.has(node)) return
    seen.add(node)
    const own = depth > 0 && !last ? [...guides, depth - 1] : guides
    if (node.row) placed.push({ row: node.row, placement: { depth, guides: own, elbow, children: node.children.length > 0 } })
    node.children.forEach((child, index) =>
      visit(child, depth + 1, own, node.row ? 'parent' : 'elsewhere', index === node.children.length - 1),
    )
  }
  sortByFirst(roots).forEach((root) => visit(root, 0, [], null, true))
  for (const node of nodes.values()) {
    if (node.row && !seen.has(node)) visit(node, 0, [], null, true)
  }
  return placed
}

function sortByFirst<T>(nodes: StackNode<T>[]): StackNode<T>[] {
  for (const node of nodes) {
    sortByFirst(node.children)
    node.first = Math.min(node.first, ...node.children.map((child) => child.first))
  }
  return nodes.sort((left, right) => left.first - right.first)
}
