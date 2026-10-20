import type { ReviewFileDiff } from '@/api/types'
import type { RequestDiscussionLineRange } from '@/api/types.generated'
import type { ReviewDiffAnnotations } from '@/features/review/review-file-diff-drawer'
import { renderedDiffLines, reviewDiffSlotName } from '@/features/review/review-file-diff-lines'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { Link } from '@tanstack/react-router'
import { MessageSquare } from 'lucide-react'
import { type ReactNode, useCallback, useMemo, useState } from 'react'
import { compactDiscussionSummary } from './discussion-preview-text'
import { runRequestContentSubmission } from './request-attachment-drafts'
import {
  canStartLineDiscussion,
  extendLineSelection,
  lineDiscussionAnchor,
  lineDiscussionDraftTarget,
  lineDiscussionsForFile,
  lineRangeLabel,
  settleLineDiscussionDraft,
  type FileLineDiscussion,
  type LineDiscussionDraft,
  type LineDiscussionComparison,
  type PlacedLineDiscussion,
} from './request-changes-line-discussions'
import type { CreateDiscussionInput } from './request-discussion-api'
import { RequestLineDiscussionComposer } from './request-discussion-composer'
import type { RequestDiscussion, RequestDiscussionMutation } from './request-discussion-types'

type RequestParams = { owner: string; repo: string; request_id: string }

export type CreateLineDiscussion = (input: CreateDiscussionInput) => Promise<RequestDiscussionMutation>

export function useLineDiscussionAnnotations({
  canOpenDiscussion,
  comparison,
  createDiscussion,
  diff,
  discussions,
  onCreated,
  params,
}: {
  canOpenDiscussion: boolean
  comparison: LineDiscussionComparison | null
  createDiscussion: CreateLineDiscussion
  diff: ReviewFileDiff | null
  discussions: RequestDiscussion[]
  onCreated: () => void
  params: RequestParams
}): ReviewDiffAnnotations | undefined {
  const comparisonKey = comparison ? JSON.stringify(comparison) : null
  const [draft, setDraft] = useState<LineDiscussionDraft | null>(null)
  const selection = draft && draft.key === comparisonKey ? draft : null
  const html = diff?.presentation.kind === 'html' ? diff.presentation.html : null
  const rendered = useMemo(() => html === null ? null : renderedDiffLines(html), [html])
  const { fileLevel, placed } = useMemo(
    () => comparison
      ? lineDiscussionsForFile(discussions, comparison, rendered)
      : { fileLevel: [], placed: [] },
    [comparison, discussions, rendered],
  )
  const selectable = Boolean(comparison && html !== null && canOpenDiscussion && canStartLineDiscussion(comparison.mode))
  const onSelectLine = useCallback((line: { side: RequestDiscussionLineRange['side']; line: number }, extend: boolean) => {
    if (!comparisonKey) return
    setDraft((current) => ({
      error: null,
      key: comparisonKey,
      lines: extendLineSelection(current?.key === comparisonKey ? current.lines : null, line, extend),
    }))
  }, [comparisonKey])
  const submit = useCallback((lines: RequestDiscussionLineRange, body: string, submissionId: string) => {
    if (!comparison || !comparisonKey) return Promise.resolve(false)
    const submitted = { key: comparisonKey, lines }
    return runRequestContentSubmission(submissionId, async () => {
      try {
        await createDiscussion({
          ...params,
          anchor: lineDiscussionAnchor(comparison, lines),
          body_markdown: body,
          client_discussion_id: submissionId,
        })
      } catch (error) {
        const message = resourceErrorMessage(error, 'Discussion could not be posted.')
        setDraft((current) => settleLineDiscussionDraft(current, submitted, { error: message }))
        return false
      }
      setDraft((current) => settleLineDiscussionDraft(current, submitted, { posted: true }))
      onCreated()
      return true
    })
  }, [comparison, comparisonKey, createDiscussion, onCreated, params])

  return useMemo(() => {
    if (!comparison) return undefined
    const composer = selection && selectable ? {
      lines: selection.lines,
      node: (
        <RequestLineDiscussionComposer
          error={selection.error}
          label={lineRangeLabel(selection.lines)}
          onCancel={() => setDraft(null)}
          onSubmit={(body, submissionId) => submit(selection.lines, body, submissionId)}
          target={lineDiscussionDraftTarget(lineDiscussionAnchor(comparison, selection.lines))}
        />
      ),
    } : null
    return {
      fileNotes: fileLevel.length ? <FileLineDiscussions discussions={fileLevel} params={params} /> : null,
      lines: lineSlots(placed, composer, params),
      onSelectLine: selectable ? onSelectLine : undefined,
      selection: selection && selectable ? selection.lines : null,
    }
  }, [comparison, fileLevel, onSelectLine, params, placed, selectable, selection, submit])
}

function lineSlots(
  placed: PlacedLineDiscussion[],
  composer: { lines: RequestDiscussionLineRange; node: ReactNode } | null,
  params: RequestParams,
): ReviewDiffAnnotations['lines'] {
  const slots = new Map<string, { side: RequestDiscussionLineRange['side']; line: number; discussions: PlacedLineDiscussion[]; composer: ReactNode }>()
  const slotFor = (lines: RequestDiscussionLineRange) => {
    const line = { line: lines.end_line, side: lines.side }
    const key = reviewDiffSlotName(line)
    const slot = slots.get(key) ?? { ...line, composer: null, discussions: [] }
    slots.set(key, slot)
    return slot
  }
  for (const item of placed) slotFor(item.lines).discussions.push(item)
  if (composer) slotFor(composer.lines).composer = composer.node
  return [...slots.values()].map(({ composer: node, discussions, line, side }) => ({
    content: (
      <div className="space-y-2">
        {discussions.length ? (
          <ul aria-label={`Discussions on ${side} line ${line}`} className="space-y-1">
            {discussions.map(({ discussion, lines }) => (
              <li key={discussion.id}>
                <LineDiscussionLink discussion={discussion} params={params}>
                  {lines.start_line === lines.end_line ? null : lineRangeLabel(lines)}
                </LineDiscussionLink>
              </li>
            ))}
          </ul>
        ) : null}
        {node}
      </div>
    ),
    line,
    side,
  }))
}

function FileLineDiscussions({
  discussions,
  params,
}: {
  discussions: FileLineDiscussion[]
  params: RequestParams
}) {
  return (
    <ul aria-label="File discussions" className="space-y-1 border-b border-border px-3 py-2 text-xs">
      {discussions.map(({ discussion, lines }) => (
        <li key={discussion.id}>
          <LineDiscussionLink discussion={discussion} params={params}>
            {lines ? `${lineRangeLabel(lines)} not shown` : 'Outdated'}
          </LineDiscussionLink>
        </li>
      ))}
    </ul>
  )
}

function LineDiscussionLink({
  children,
  discussion,
  params,
}: {
  children: ReactNode
  discussion: RequestDiscussion
  params: RequestParams
}) {
  return (
    <Link
      className="flex min-w-0 items-center gap-2 text-foreground underline-offset-4 hover:underline"
      hash={`discussion-${discussion.id}`}
      params={{ owner: params.owner, repo: params.repo, requestId: params.request_id }}
      search={{ discussion: discussion.id }}
      to="/$owner/$repo/requests/$requestId"
    >
      <MessageSquare aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground" />
      <span className="truncate">{compactDiscussionSummary(discussion.body_markdown)}</span>
      <span className="ml-auto flex shrink-0 gap-2 text-muted-foreground">
        {children ? <span>{children}</span> : null}
        <span>{discussion.status === 'Resolved' ? 'Resolved' : 'Open'}</span>
      </span>
    </Link>
  )
}
