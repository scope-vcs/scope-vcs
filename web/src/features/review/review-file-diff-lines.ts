import type { RequestDiffSide } from '../../api/types.generated'

export type ReviewDiffLine = { side: RequestDiffSide; line: number }

export type ReviewDiffLineRange = { side: RequestDiffSide; start_line: number; end_line: number }

export type ReviewDiffRenderedLines = Record<RequestDiffSide, ReadonlySet<number>>

const ANNOTATION_ATTRIBUTE = 'data-scope-annotation'
const CONTENT_ROW = /<div\b[^>]*\sdata-line-type="[^"]+"[^>]*>/g

export function renderedDiffLines(html: string): ReviewDiffRenderedLines {
  const lines = { new: new Set<number>(), old: new Set<number>() }
  for (const [tag] of html.matchAll(CONTENT_ROW)) {
    const row = rowLines(
      attribute(tag, 'data-line-type'),
      attribute(tag, 'data-line'),
      attribute(tag, 'data-alt-line'),
    )
    if (row.new !== null) lines.new.add(row.new)
    if (row.old !== null) lines.old.add(row.old)
  }
  return lines
}

export function reviewDiffSlotName({ side, line }: ReviewDiffLine) {
  return `scope-line-${side}-${line}`
}

export function diffLineAt(target: EventTarget | null): ReviewDiffLine | null {
  if (!(target instanceof Element)) return null
  const cell = target.closest('[data-column-number]')
  const line = Number(cell?.getAttribute('data-column-number'))
  if (!cell || !Number.isSafeInteger(line) || line < 1) return null
  return {
    line,
    side: cell.getAttribute('data-line-type') === 'change-deletion' ? 'old' : 'new',
  }
}

export function syncDiffAnnotationRows(root: ParentNode, lines: ReviewDiffLine[]) {
  for (const row of root.querySelectorAll(`[${ANNOTATION_ATTRIBUTE}]`)) {
    adjustRowSpan(row.parentElement, -1)
    row.remove()
  }
  for (const line of lines) {
    const rows = diffRows(root, line)
    if (!rows) continue
    const annotation = document.createElement('div')
    annotation.setAttribute('data-line-annotation', '')
    annotation.setAttribute(ANNOTATION_ATTRIBUTE, '')
    const content = document.createElement('div')
    content.setAttribute('data-annotation-content', '')
    const slot = document.createElement('slot')
    slot.name = reviewDiffSlotName(line)
    content.append(slot)
    annotation.append(content)
    const buffer = document.createElement('div')
    buffer.setAttribute('data-gutter-buffer', 'annotation')
    buffer.setAttribute(ANNOTATION_ATTRIBUTE, '')
    rows.content.after(annotation)
    rows.gutter?.after(buffer)
    adjustRowSpan(rows.content.parentElement, 1)
    adjustRowSpan(rows.gutter?.parentElement ?? null, 1)
  }
}

export function markSelectedDiffLines(root: ParentNode, range: ReviewDiffLineRange | null) {
  for (const row of root.querySelectorAll('[data-selected-line]')) {
    row.removeAttribute('data-selected-line')
  }
  if (!range) return
  for (let line = range.start_line; line <= range.end_line; line++) {
    const rows = diffRows(root, { line, side: range.side })
    rows?.content.setAttribute('data-selected-line', '')
    rows?.gutter?.setAttribute('data-selected-line', '')
  }
}

function diffRows(root: ParentNode, target: ReviewDiffLine) {
  for (const content of root.querySelectorAll('[data-content] > [data-line-type]')) {
    const row = rowLines(
      content.getAttribute('data-line-type'),
      content.getAttribute('data-line'),
      content.getAttribute('data-alt-line'),
    )
    if (row[target.side] !== target.line) continue
    const index = content.getAttribute('data-line-index')
    const gutter = index
      ? root.querySelector(`[data-gutter] > [data-line-index="${CSS.escape(index)}"]`)
      : null
    return { content, gutter }
  }
  return null
}

function rowLines(type: string | null, line: string | null, altLine: string | null) {
  const primary = lineNumber(line)
  if (type === 'change-deletion') return { new: null, old: primary }
  if (type === 'change-addition') return { new: primary, old: null }
  return { new: primary, old: lineNumber(altLine) }
}

function lineNumber(value: string | null) {
  const line = Number(value)
  return value && Number.isSafeInteger(line) && line > 0 ? line : null
}

function attribute(tag: string, name: string) {
  return new RegExp(`\\s${name}="([^"]*)"`).exec(tag)?.[1] ?? null
}

function adjustRowSpan(column: Element | null, delta: number) {
  if (!(column instanceof HTMLElement)) return
  const span = Number(/span\s+(\d+)/.exec(column.style.gridRow)?.[1])
  if (Number.isSafeInteger(span)) column.style.gridRow = `span ${span + delta}`
}
