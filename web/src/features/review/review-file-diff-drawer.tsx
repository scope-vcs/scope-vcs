import type {
  ReviewDiffBinarySide,
  ReviewDiffTextSide,
  ReviewFileDiff,
} from '@/api/types'
import { PanelState } from '@/components/empty-state'
import { normalizeFilePath } from '@/components/file-system-tree-model'
import { PendingSurface } from '@/components/pending-surface'
import { Button } from '@/components/ui/button'
import { formatBytes } from '@/lib/format-bytes'
import { shortOid } from '@/lib/short-oid'
import { DiffSkeleton } from './diff-skeleton'
import DOMPurify from 'dompurify'
import { File, FileText, TriangleAlert, X } from 'lucide-react'
import { type ReactNode, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import {
  reviewFileDiffEmptyLabel,
  reviewFileDiffModeChangeLabel,
  reviewFileDiffOmittedLabel,
} from './review-file-diff-presentation'
import {
  diffLineAt,
  markSelectedDiffLines,
  reviewDiffSlotName,
  syncDiffAnnotationRows,
  type ReviewDiffLine,
  type ReviewDiffLineRange,
} from './review-file-diff-lines'

export type ReviewDiffAnnotations = {
  fileNotes: ReactNode
  lines: Array<ReviewDiffLine & { content: ReactNode }>
  onSelectLine?: (line: ReviewDiffLine, extend: boolean) => void
  selection: ReviewDiffLineRange | null
}

export function ReviewFileDiffDrawer({
  actions,
  annotations,
  cacheKey,
  context,
  diff,
  error,
  loading,
  onClose,
  onRetry,
  onScrollTopChange,
  scrollTop = 0,
  selectedPath,
}: {
  actions?: ReactNode
  annotations?: ReviewDiffAnnotations
  cacheKey?: string | null
  context?: ReactNode
  diff: ReviewFileDiff | null
  error: string | null
  loading: boolean
  onClose?: () => void
  onRetry?: () => void
  onScrollTopChange?: (scrollTop: number) => void
  scrollTop?: number
  selectedPath: string | null
}) {
  const displayName = normalizeFilePath(diff?.path ?? selectedPath ?? '')
  const scrollRef = useRef<HTMLDivElement>(null)
  const restoredScrollKeyRef = useRef<string | null>(null)
  const scrollKey = cacheKey ?? selectedPath ?? null

  useLayoutEffect(() => {
    if (restoredScrollKeyRef.current === scrollKey) return
    restoredScrollKeyRef.current = scrollKey
    if (scrollRef.current) scrollRef.current.scrollTop = scrollTop
  })

  return (
    <aside
      aria-label={displayName ? `${displayName} diff` : 'File diff'}
      className="h-full min-h-[340px] bg-background"
    >
      <div className="flex h-full min-h-0 flex-col">
        <div className="flex min-h-14 items-center gap-3 border-b border-border px-3 py-2.5">
          <FileText className="size-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0 flex-1">
            <div
              className="truncate font-mono text-xs font-medium leading-5"
              title={displayName}
            >
              {displayName || 'Diff'}
            </div>
            <div className="text-xs leading-4 text-muted-foreground">
              {loading
                ? null
                : error
                  ? 'Diff unavailable'
                  : reviewFileDiffModeChangeLabel(diff) ?? 'Diff'}
            </div>
          </div>
          {actions}
          {onClose && (
            <Button
              aria-label="Close diff viewer"
              onClick={onClose}
              size="icon-xs"
              type="button"
              variant="ghost"
            >
              <X className="size-3.5" />
            </Button>
          )}
        </div>
        {context}

        <div
          className="min-h-0 flex-1 overflow-auto"
          onScroll={(event) => onScrollTopChange?.(event.currentTarget.scrollTop)}
          ref={scrollRef}
        >
          {loading ? (
            <PendingSurface
              className="min-h-full"
              label={`Loading ${displayName || 'file'} diff`}
              onRetry={onRetry}
            >
              <DiffSkeleton />
            </PendingSurface>
          ) : error ? (
            <PanelState role="alert" tone="error">
              <TriangleAlert className="size-5" />
              <span>{error}</span>
              {onRetry && (
                <Button onClick={onRetry} size="sm" type="button" variant="secondary">
                  Retry
                </Button>
              )}
            </PanelState>
          ) : (
            <>
              {annotations?.fileNotes}
              {diff?.presentation.kind === 'html'
                ? <PrerenderedDiff annotations={annotations} html={diff.presentation.html} />
                : <ReviewFileDiffFallback diff={diff} />}
            </>
          )}
        </div>
      </div>
    </aside>
  )
}

function ReviewFileDiffFallback({ diff }: { diff: ReviewFileDiff | null }) {
  return diff?.presentation.kind === 'mixed' ? (
    <MixedContentDiffState
      binary={diff.presentation.binary}
      text={diff.presentation.text}
    />
  ) : diff?.presentation.kind === 'binary' ? (
    <BinaryDiffState sides={diff.presentation.sides} />
  ) : diff?.presentation.kind === 'omitted' ? (
    <OmittedDiffState reason={diff.presentation.reason} />
  ) : (
    <PanelState>
      <FileText className="size-5" />
      <span>{reviewFileDiffEmptyLabel(diff)}</span>
    </PanelState>
  )
}

function PrerenderedDiff({
  annotations,
  html,
}: {
  annotations?: ReviewDiffAnnotations
  html: string
}) {
  const containerRef = useRef<HTMLDivElement>(null)
  const [host, setHost] = useState<HTMLElement | null>(null)
  const onSelectLine = annotations?.onSelectLine
  const selectLineRef = useRef(onSelectLine)
  const lines = annotations?.lines
  const lineKeys = JSON.stringify(lines?.map(({ line, side }) => ({ line, side })) ?? [])
  const selection = annotations?.selection ?? null
  const selectable = Boolean(onSelectLine)

  useLayoutEffect(() => {
    selectLineRef.current = onSelectLine
  })

  useLayoutEffect(() => {
    const container = containerRef.current
    if (!container) return

    const fragment = DOMPurify.sanitize(html, {
      RETURN_DOM_FRAGMENT: true,
    })
    const nextHost = document.createElement('diffs-container')
    const root = nextHost.attachShadow({ mode: 'open' })
    root.replaceChildren(fragment)
    root.addEventListener('click', (event) => {
      if (!(event instanceof MouseEvent)) return
      const line = diffLineAt(event.target)
      if (!line || !selectLineRef.current) return
      event.preventDefault()
      selectLineRef.current(line, event.shiftKey)
    })
    container.replaceChildren(nextHost)
    setHost(nextHost)
  }, [html])

  useLayoutEffect(() => {
    const root = host?.shadowRoot
    if (!root) return
    root.querySelector('[data-diff]')?.toggleAttribute('data-interactive-line-numbers', selectable)
    syncDiffAnnotationRows(root, JSON.parse(lineKeys) as ReviewDiffLine[])
    markSelectedDiffLines(root, selection)
  }, [host, lineKeys, selectable, selection])

  return (
    <>
      <div
        className="review-diff-viewer scope-content-enter"
        ref={containerRef}
        suppressHydrationWarning
      />
      {host && lines?.length ? createPortal(
        lines.map((line) => (
          <div
            className="px-3 py-2 font-sans text-xs whitespace-normal"
            key={reviewDiffSlotName(line)}
            slot={reviewDiffSlotName(line)}
          >
            {line.content}
          </div>
        )),
        host,
      ) : null}
    </>
  )
}

function BinaryDiffState({ sides }: { sides: ReviewDiffBinarySide[] }) {
  return (
    <PanelState>
      <BinarySummary sides={sides} />
    </PanelState>
  )
}

function MixedContentDiffState({
  binary,
  text,
}: {
  binary: ReviewDiffBinarySide[]
  text: ReviewDiffTextSide[]
}) {
  return (
    <div className="min-h-[220px]">
      <div className="border-b border-border px-4 py-4 text-sm text-muted-foreground">
        <BinarySummary sides={binary} />
      </div>
      {text.map((side) => (
        <section key={side.label}>
          <div className="border-b border-border px-4 py-2 text-xs font-medium text-muted-foreground">
            {side.label} text
          </div>
          <pre className="overflow-auto whitespace-pre-wrap break-words px-4 py-3 font-mono text-xs leading-5 text-foreground">
            {side.content || 'Empty text file'}
          </pre>
          {side.truncated && (
            <div className="border-t border-border px-4 py-2 text-xs text-muted-foreground">
              Text preview truncated
            </div>
          )}
        </section>
      ))}
    </div>
  )
}

function BinarySummary({ sides }: { sides: ReviewDiffBinarySide[] }) {
  return (
    <div className="w-full max-w-md space-y-3">
      <div className="flex items-center gap-2 text-foreground">
        <File className="size-4" />
        <span className="font-medium">Binary file not rendered</span>
      </div>
      <div className="space-y-2 font-mono text-xs leading-5">
        {sides.map((side) => (
          <div
            className="grid grid-cols-[44px_minmax(0,1fr)] gap-x-3"
            key={`${side.label}-${side.oid}`}
          >
            <span className="text-muted-foreground">{side.label}</span>
            <span className="min-w-0 break-all">
              {formatBytes(side.sizeBytes)} - {shortOid(side.oid)}
            </span>
          </div>
        ))}
      </div>
    </div>
  )
}

function OmittedDiffState({
  reason,
}: {
  reason: Extract<ReviewFileDiff['presentation'], { kind: 'omitted' }>['reason']
}) {
  return (
    <PanelState>
      <FileText className="size-5" />
      <span>{reviewFileDiffOmittedLabel(reason)}</span>
    </PanelState>
  )
}

