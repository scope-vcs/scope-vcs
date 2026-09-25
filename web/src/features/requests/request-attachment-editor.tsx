import type { RequestAttachmentLimitsResponse } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { formatBytes } from '@/lib/format-bytes'
import { cn } from '@/lib/utils'
import { FileImage, FileVideo, Paperclip, RotateCcw, X } from 'lucide-react'
import {
  type ClipboardEvent,
  type DragEvent,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import {
  addRequestAttachmentDraftFiles,
  beginRequestAttachmentSubmission,
  clearRequestAttachmentDraft,
  finishRequestAttachmentSubmission,
  inferredMediaType,
  readRequestAttachmentDraft,
  requestAttachmentDraftKey,
  requestAttachmentMarkdownReference,
  seedRequestAttachmentDraft,
  setRequestAttachmentDraftText,
  subscribeRequestAttachmentDraft,
  type DraftAttachment,
  type RequestAttachmentDraftTarget,
} from './request-attachment-drafts'
import { useRequestAttachments } from './request-attachment-context'
import { refreshRequestAttachments } from './request-attachment-resource'
import {
  attachmentTargetForDraft,
  removeUploadingRequestAttachment,
  uploadRequestAttachment,
} from './request-attachment-upload'
import {
  insertRequestAttachmentReferences,
  requestAttachmentDraftReference,
  requestAttachmentContentCount,
} from './request-attachment-reference'

const staleDescriptionMessage = 'The description changed while you were editing. Your draft is kept.'

export function RequestAttachmentEditor({
  autoFocus = false,
  enterSubmits = true,
  error = null,
  initialText = '',
  label,
  minHeight,
  onCancel,
  onCancelQuote,
  onSubmit,
  placeholder,
  quote,
  secondarySubmit,
  submitIcon,
  submitLabel,
  target,
}: {
  autoFocus?: boolean
  enterSubmits?: boolean
  /** A failure the caller reports, such as a rejected save. */
  error?: string | null
  initialText?: string
  label: string
  /** Starting height, so an editor that replaces rendered text keeps its size. */
  minHeight?: number
  onCancel: () => void
  onCancelQuote?: () => void
  onSubmit: (markdown: string, baseText: string | null, submissionId: string) => Promise<boolean>
  placeholder: string
  quote?: { author: string; body: string } | null
  secondarySubmit?: {
    icon: ReactNode
    label: string
    onSubmit: (markdown: string, baseText: string | null, submissionId: string) => Promise<boolean>
  }
  submitIcon: ReactNode
  submitLabel: string
  target: RequestAttachmentDraftTarget
}) {
  const environment = useRequestAttachments()
  const editorId = useId()
  const formId = `${editorId}-form`
  const fileInputRef = useRef<HTMLInputElement>(null)
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const [pendingAction, setPendingAction] = useState<'primary' | 'secondary' | null>(null)
  const [dragging, setDragging] = useState(false)
  const [validationError, setValidationError] = useState<string | null>(null)
  const draftKey = requestAttachmentDraftKey({
    accessScope: environment.accessScope,
    repoId: environment.repoId,
    requestId: environment.requestId,
    target,
    viewerId: environment.viewerId,
  })
  const subscribe = useCallback(
    (listener: () => void) => subscribeRequestAttachmentDraft(draftKey, listener),
    [draftKey],
  )
  const read = useCallback(() => readRequestAttachmentDraft(draftKey), [draftKey])
  const draft = useSyncExternalStore(subscribe, read, read)
  const editsDescription = target === 'description'
  const staleDescription = editsDescription && draft.initialized && draft.baseText !== initialText
  const limits = environment.limits
  const acceptedMedia = limits
    ? [...limits.accepted_photo_media_types, ...limits.accepted_video_media_types]
    : []

  useEffect(() => {
    seedRequestAttachmentDraft(draftKey, initialText)
  }, [draftKey, initialText])

  useFitToContent(textareaRef, draft.text)

  const readyAttachments = draft.attachments.filter(
    (attachment) => attachment.status === 'uploaded' && attachment.attachmentId,
  )
  const transfersReady = draft.attachments.every(
    (attachment) => attachment.status === 'uploaded',
  )
  const transferPending = draft.attachments.some(
    (attachment) => attachment.status !== 'uploaded',
  )
  const hasFailedTransfer = draft.attachments.some(
    (attachment) => attachment.status === 'failed',
  )
  const attachmentCount = requestAttachmentContentCount(draft.text, draft.attachments)
  const pending = pendingAction !== null || draft.pending
  const overLimit = limits !== null && attachmentCount > limits.max_attachments_per_content
  const canSubmit = !pending && !staleDescription && transfersReady && !overLimit && (
    editsDescription || Boolean(draft.text.trim()) || readyAttachments.length > 0
  )
  const status = editorStatus({
    enterSubmits,
    hasFailedTransfer,
    overLimitBy: overLimit ? limits.max_attachments_per_content : null,
    transferPending,
  })
  const alert = validationError ?? error ?? (staleDescription ? staleDescriptionMessage : null)
  const draftChanged = staleDescription || draft.text !== initialText || draft.attachments.length > 0

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!canSubmit) return
    const submitter = (event.nativeEvent as SubmitEvent)
      .submitter as HTMLButtonElement | null
    const action = submitter?.dataset.submitAction === 'secondary'
      ? 'secondary'
      : 'primary'
    const submitAction = action === 'secondary'
      ? secondarySubmit?.onSubmit
      : onSubmit
    if (!submitAction) return
    const markdown = markdownWithAttachments(draft.text, readyAttachments)
    const submissionId = beginRequestAttachmentSubmission(
      draftKey,
      JSON.stringify([action, markdown, draft.baseText]),
    )
    if (!submissionId) return
    setPendingAction(action)
    let posted = false
    try {
      posted = await submitAction(markdown, draft.baseText, submissionId)
      if (posted) onCancelQuote?.()
    } finally {
      finishRequestAttachmentSubmission(submissionId, posted)
      setPendingAction(null)
    }
  }

  function addFiles(files: File[]) {
    if (readRequestAttachmentDraft(draftKey).pending) return
    if (!limits) {
      setValidationError(environment.attachmentsError ?? 'Attachment limits are still loading.')
      return
    }
    const resumableCount = draft.attachments.filter((attachment) => attachment.file === null && files.some((file) =>
      attachment.name === (file.name || 'Pasted image') && attachment.size === file.size,
    )).length
    const accepted = validateFiles(files, attachmentCount - resumableCount, limits)
    setValidationError(accepted.error)
    const attachments = addRequestAttachmentDraftFiles(draftKey, accepted.files)
    const unplaced = attachments.filter((attachment) =>
      !draft.text.includes(`](${requestAttachmentMarkdownReference(attachment.localId)})`),
    )
    if (unplaced.length > 0) {
      const textarea = textareaRef.current
      const cursor = textarea?.selectionStart ?? draft.text.length
      setRequestAttachmentDraftText(
        draftKey,
        insertRequestAttachmentReferences(
          draft.text,
          cursor,
          unplaced.map(requestAttachmentDraftReference),
        ),
      )
    }
    for (const attachment of attachments) upload(attachment.localId)
  }

  function upload(localId: string) {
    void uploadRequestAttachment({
      actions: environment.actions,
      draftKey,
      localId,
      params: environment.params,
      target: attachmentTargetForDraft(target),
      onCompleted: () => refreshRequestAttachments(environment.accessScope, environment.requestId),
    })
  }

  function handlePaste(event: ClipboardEvent<HTMLTextAreaElement>) {
    const files: File[] = []
    for (const item of event.clipboardData.items) {
      if (item.kind !== 'file') continue
      const file = item.getAsFile()
      if (file) files.push(file)
    }
    if (files.length === 0) return
    event.preventDefault()
    addFiles(files)
  }

  function handleDrop(event: DragEvent<HTMLDivElement>) {
    event.preventDefault()
    setDragging(false)
    addFiles([...event.dataTransfer.files])
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (pending || event.nativeEvent.isComposing) return
    if (event.key === 'Escape') {
      event.preventDefault()
      if (quote && onCancelQuote) onCancelQuote()
      else onCancel()
      return
    }
    if (event.key !== 'Enter' || event.shiftKey) return
    if (!enterSubmits && !event.metaKey && !event.ctrlKey) return
    event.preventDefault()
    event.currentTarget.form?.requestSubmit()
  }

  const actions = (
    <EditorActions
      canSubmit={canSubmit}
      formId={formId}
      limitsReady={limits !== null}
      onAttach={() => fileInputRef.current?.click()}
      onCancel={onCancel}
      onDiscardDraft={editsDescription && draftChanged
        ? () => {
            clearRequestAttachmentDraft(draftKey)
            seedRequestAttachmentDraft(draftKey, initialText)
            setValidationError(null)
          }
        : undefined}
      pending={pending}
      pendingAction={pendingAction}
      secondarySubmit={secondarySubmit}
      status={status}
      submitIcon={submitIcon}
      submitLabel={submitLabel}
    />
  )

  return (
    <form id={formId} onSubmit={submit}>
      <label className="sr-only" htmlFor={editorId}>{label}</label>
      {quote ? <QuotedReply disabled={pending} onCancel={onCancelQuote} quote={quote} /> : null}
      <div
        className={cn(
          'transition-colors',
          editsDescription
            ? '-mx-2 bg-muted/40 px-2 shadow-[inset_2px_0_0_var(--color-ring)]'
            : 'overflow-hidden rounded-md border border-input bg-background',
          dragging && 'ring-3 ring-ring/30',
        )}
        onDragEnter={(event) => { event.preventDefault(); setDragging(true) }}
        onDragLeave={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDragging(false)
        }}
        onDragOver={(event) => event.preventDefault()}
        onDrop={handleDrop}
      >
        <textarea
          autoFocus={autoFocus}
          className={cn(
            'block w-full resize-none bg-transparent outline-none placeholder:text-muted-foreground disabled:cursor-wait disabled:opacity-70',
            editsDescription ? 'text-base leading-[26px]' : 'min-h-28 px-3 py-2 text-sm leading-6',
          )}
          disabled={pending}
          id={editorId}
          onChange={(event) => setRequestAttachmentDraftText(draftKey, event.target.value)}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
          placeholder={placeholder}
          ref={textareaRef}
          style={minHeight === undefined ? undefined : { minHeight }}
          value={draft.text}
        />
        {draft.attachments.length > 0 ? (
          <div className={cn('pb-2', !editsDescription && 'px-3')}>
            {draft.attachments.map((attachment) => (
              <DraftAttachmentRow
                attachment={attachment}
                disabled={pending}
                key={attachment.localId}
                onRemove={() => removeUploadingRequestAttachment(draftKey, attachment.localId)}
                onRetry={() => upload(attachment.localId)}
                onReselect={() => fileInputRef.current?.click()}
              />
            ))}
          </div>
        ) : null}
      </div>
      <input
        accept={acceptedMedia.join(',')}
        aria-label="Attach photos or videos"
        className="sr-only"
        disabled={pending}
        multiple
        onChange={(event) => {
          addFiles([...(event.target.files ?? [])])
          event.target.value = ''
        }}
        ref={fileInputRef}
        type="file"
      />
      {alert ? <p className="mt-2 text-sm text-destructive" role="alert">{alert}</p> : null}
      {staleDescription ? <StaleDescriptionNotice currentDescription={initialText} /> : null}
      {actions}
    </form>
  )
}

function EditorActions({
  canSubmit,
  formId,
  limitsReady,
  onAttach,
  onCancel,
  onDiscardDraft,
  pending,
  pendingAction,
  secondarySubmit,
  status,
  submitIcon,
  submitLabel,
}: {
  canSubmit: boolean
  formId: string
  limitsReady: boolean
  onAttach: () => void
  onCancel: () => void
  /** Present where a kept draft can be swapped for the saved text. */
  onDiscardDraft?: () => void
  pending: boolean
  pendingAction: 'primary' | 'secondary' | null
  secondarySubmit?: { icon: ReactNode; label: string }
  status: string
  submitIcon: ReactNode
  submitLabel: string
}) {
  return (
    <div className="mt-2 flex flex-wrap items-center gap-2">
      <Button
        aria-label="Attach files"
        disabled={!limitsReady || pending}
        onClick={onAttach}
        size="icon-sm"
        title="Attach files. You can also drop or paste them."
        type="button"
        variant="ghost"
      >
        <Paperclip />
      </Button>
      <p aria-live="polite" className="min-w-0 flex-1 text-xs text-muted-foreground">{status}</p>
      <div className="ml-auto flex flex-wrap items-center justify-end gap-2">
        {onDiscardDraft ? (
          <Button
            disabled={pending}
            onClick={onDiscardDraft}
            size="sm"
            title="Discard this draft and load the current description"
            type="button"
            variant="ghost"
          >
            Discard draft
          </Button>
        ) : null}
        <Button disabled={pending} onClick={onCancel} size="sm" type="button" variant="ghost">Cancel</Button>
        {secondarySubmit ? (
          <Button
            data-submit-action="secondary"
            disabled={!canSubmit}
            form={formId}
            size="sm"
            type="submit"
            variant="secondary"
          >
            {secondarySubmit.icon}
            {pendingAction === 'secondary' ? 'Saving…' : secondarySubmit.label}
          </Button>
        ) : null}
        <Button disabled={!canSubmit} form={formId} size="sm" title="Ctrl+Enter or ⌘+Enter" type="submit">
          {submitIcon}
          {pendingAction === 'primary' ? 'Saving…' : submitLabel}
        </Button>
      </div>
    </div>
  )
}

function QuotedReply({
  disabled,
  onCancel,
  quote,
}: {
  disabled: boolean
  onCancel?: () => void
  quote: { author: string; body: string }
}) {
  return (
    <div className="mb-2 flex min-w-0 items-start gap-2 border-l-2 border-border-strong pl-3 text-xs leading-5 text-muted-foreground">
      <div className="min-w-0 flex-1">
        <span className="font-medium text-foreground">{quote.author}</span>
        <span className="ml-1 line-clamp-1">{quote.body}</span>
      </div>
      <button aria-label="Cancel quoted reply" className="shrink-0 p-1 hover:text-foreground" disabled={disabled} onClick={onCancel} type="button">
        <X className="size-3.5" />
      </button>
    </div>
  )
}

function StaleDescriptionNotice({ currentDescription }: { currentDescription: string }) {
  return (
    <details className="mt-2 text-sm">
      <summary className="cursor-pointer">Current description</summary>
      <pre className="mt-2 whitespace-pre-wrap break-words font-sans">
        {currentDescription || 'No description.'}
      </pre>
    </details>
  )
}

function DraftAttachmentRow({
  attachment,
  disabled,
  onRemove,
  onReselect,
  onRetry,
}: {
  attachment: DraftAttachment
  disabled: boolean
  onRemove: () => void
  onReselect: () => void
  onRetry: () => void
}) {
  const mediaLabel = attachment.contentType.startsWith('video/') ? 'Video' : 'Photo'
  return (
    <div className="grid grid-cols-[2rem_minmax(0,1fr)_auto] items-center gap-2 border-t border-border py-2 first:border-t-0">
      {attachment.previewUrl ? (
        <img alt="" className="size-8 rounded object-cover" src={attachment.previewUrl} />
      ) : attachment.contentType.startsWith('video/') ? (
        <FileVideo aria-hidden="true" className="size-5 text-muted-foreground" />
      ) : (
        <FileImage aria-hidden="true" className="size-5 text-muted-foreground" />
      )}
      <div className="min-w-0">
        <p className="truncate text-xs font-medium">{attachment.name}</p>
        <p className={cn('text-[11px] text-muted-foreground', attachment.status === 'failed' && 'text-destructive')}>
          {attachment.status === 'failed'
            ? attachment.error
            : attachment.status === 'uploaded'
              ? `${mediaLabel} uploaded · preparing preview`
              : `${formatBytes(attachment.size)} · ${Math.round(attachment.progress * 100)}%`}
        </p>
        {attachment.status === 'uploading' ? (
          <progress
            aria-label={`${attachment.name}: ${Math.round(attachment.progress * 100)}% uploaded`}
            className="mt-1 block h-1 w-full max-w-60 appearance-none overflow-hidden rounded-full bg-muted [&::-moz-progress-bar]:bg-foreground [&::-webkit-progress-bar]:bg-muted [&::-webkit-progress-value]:bg-foreground"
            max={1}
            value={attachment.progress}
          />
        ) : null}
      </div>
      <div className="flex items-center">
        {attachment.status === 'failed' ? (
          <Button
            disabled={disabled}
            aria-label={attachment.file ? `Retry ${attachment.name}` : `Select ${attachment.name} again`}
            onClick={attachment.file ? onRetry : onReselect}
            size="icon-sm"
            title={attachment.file ? 'Retry upload' : 'Select the same file to resume'}
            type="button"
            variant="ghost"
          ><RotateCcw /></Button>
        ) : null}
        <Button disabled={disabled} aria-label={`Remove ${attachment.name}`} onClick={onRemove} size="icon-sm" title="Remove attachment" type="button" variant="ghost"><X /></Button>
      </div>
    </div>
  )
}

/** Grows the textarea with its text, so it never scrolls inside itself. */
function useFitToContent(ref: RefObject<HTMLTextAreaElement | null>, text: string) {
  useLayoutEffect(() => {
    if (ref.current) fitToContent(ref.current)
  }, [ref, text])
  // Rewrapped text changes the needed height without changing the text.
  useEffect(() => {
    const textarea = ref.current
    if (!textarea) return
    let width = textarea.clientWidth
    const observer = new ResizeObserver(() => {
      if (textarea.clientWidth === width) return
      width = textarea.clientWidth
      fitToContent(textarea)
    })
    observer.observe(textarea)
    return () => observer.disconnect()
  }, [ref])
}

function fitToContent(textarea: HTMLTextAreaElement) {
  textarea.style.height = 'auto'
  textarea.style.height = `${textarea.scrollHeight}px`
}

function editorStatus({
  enterSubmits,
  hasFailedTransfer,
  overLimitBy,
  transferPending,
}: {
  enterSubmits: boolean
  hasFailedTransfer: boolean
  /** The attachment limit, when the draft exceeds it. */
  overLimitBy: number | null
  transferPending: boolean
}) {
  if (overLimitBy !== null) return `You can attach up to ${overLimitBy} files here.`
  if (hasFailedTransfer) return 'Remove or retry failed files before saving.'
  if (transferPending) return 'Uploading files…'
  return enterSubmits ? 'Shift+Enter for a new line' : ''
}

function markdownWithAttachments(text: string, attachments: DraftAttachment[]) {
  const trimmed = text.trim()
  const references = attachments.flatMap((attachment) => {
    if (!attachment.attachmentId) return []
    const path = requestAttachmentMarkdownReference(attachment.attachmentId)
    if (trimmed.includes(`](${path})`)) return []
    return [requestAttachmentDraftReference({
      contentType: attachment.contentType,
      localId: attachment.attachmentId,
      name: attachment.name,
    })]
  })
  return [trimmed, ...references].filter(Boolean).join('\n\n')
}

function validateFiles(
  files: File[],
  currentCount: number,
  limits: RequestAttachmentLimitsResponse,
) {
  const available = Math.max(0, limits.max_attachments_per_content - currentCount)
  const accepted: File[] = []
  let error: string | null = files.length > available
    ? `You can attach up to ${limits.max_attachments_per_content} files here.`
    : null
  const photoTypes = new Set(limits.accepted_photo_media_types)
  const videoTypes = new Set(limits.accepted_video_media_types)
  for (const file of files.slice(0, available)) {
    const mediaType = inferredMediaType(file)
    const isPhoto = photoTypes.has(mediaType)
    const isVideo = videoTypes.has(mediaType)
    if (!isPhoto && !isVideo) {
      error = `${file.name} is not a supported photo or video.`
      continue
    }
    const maxBytes = isVideo ? limits.max_video_bytes : limits.max_photo_bytes
    if (file.size > maxBytes) {
      error = `${file.name} is larger than the ${formatBytes(maxBytes)} limit.`
      continue
    }
    accepted.push(file)
  }
  return { error, files: accepted }
}
