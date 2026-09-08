import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { FileImage, FileVideo, Paperclip, RotateCcw, X } from 'lucide-react'
import {
  type ClipboardEvent,
  type DragEvent,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import {
  addRequestAttachmentDraftFiles,
  clearRequestAttachmentDraft,
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

const FALLBACK_ACCEPTED_MEDIA = [
  'image/png', 'image/jpeg', 'image/webp', 'image/gif', 'image/heic', 'image/heif',
  'video/mp4', 'video/quicktime', 'video/webm',
]

export function RequestAttachmentEditor({
  autoFocus = false,
  enterSubmits = true,
  initialText = '',
  label,
  onCancel,
  onCancelQuote,
  onSubmit,
  placeholder,
  quote,
  submitIcon,
  submitLabel,
  target,
}: {
  autoFocus?: boolean
  enterSubmits?: boolean
  initialText?: string
  label: string
  onCancel: () => void
  onCancelQuote?: () => void
  onSubmit: (markdown: string, baseText: string | null) => Promise<boolean>
  placeholder: string
  quote?: { author: string; body: string } | null
  submitIcon: ReactNode
  submitLabel: string
  target: RequestAttachmentDraftTarget
}) {
  const environment = useRequestAttachments()
  const editorId = useId()
  const fileInputRef = useRef<HTMLInputElement>(null)
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const [pending, setPending] = useState(false)
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
  const acceptedMedia = environment.limits
    ? [
        ...environment.limits.accepted_photo_media_types,
        ...environment.limits.accepted_video_media_types,
      ]
    : FALLBACK_ACCEPTED_MEDIA

  useEffect(() => {
    seedRequestAttachmentDraft(draftKey, initialText)
  }, [draftKey, initialText])

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
  const attachmentLimit = environment.limits?.max_attachments_per_content ?? 10
  const canSubmit = !pending && transfersReady && attachmentCount <= attachmentLimit && (
    target === 'description' || Boolean(draft.text.trim()) || readyAttachments.length > 0
  )

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!canSubmit) return
    setPending(true)
    try {
      const posted = await onSubmit(
        markdownWithAttachments(draft.text, readyAttachments),
        draft.baseText,
      )
      if (posted) {
        clearRequestAttachmentDraft(draftKey)
        onCancelQuote?.()
      }
    } finally {
      setPending(false)
    }
  }

  function addFiles(files: File[]) {
    const resumableCount = draft.attachments.filter((attachment) => attachment.file === null && files.some((file) =>
      attachment.name === (file.name || 'Pasted image') && attachment.size === file.size,
    )).length
    const accepted = validateFiles(
      files,
      attachmentCount - resumableCount,
      environment.limits,
    )
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
    for (const attachment of attachments) {
      void uploadRequestAttachment({
        actions: environment.actions,
        draftKey,
        localId: attachment.localId,
        params: environment.params,
        target: attachmentTargetForDraft(target),
        onCompleted: () => refreshRequestAttachments(environment.accessScope, environment.requestId),
      })
    }
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
    if (event.nativeEvent.isComposing) return
    if (event.key === 'Escape') {
      event.preventDefault()
      if (quote && onCancelQuote) onCancelQuote()
      else onCancel()
      return
    }
    if (!enterSubmits || pending || event.key !== 'Enter' || event.shiftKey) return
    event.preventDefault()
    event.currentTarget.form?.requestSubmit()
  }

  return (
    <form onSubmit={submit}>
      <label className="sr-only" htmlFor={editorId}>{label}</label>
      {quote ? (
        <div className="mb-2 flex min-w-0 items-start gap-2 border-l-2 border-border-strong pl-3 text-xs leading-5 text-muted-foreground">
          <div className="min-w-0 flex-1">
            <span className="font-medium text-foreground">{quote.author}</span>
            <span className="ml-1 line-clamp-1">{quote.body}</span>
          </div>
          <button aria-label="Cancel quoted reply" className="shrink-0 p-1 hover:text-foreground" onClick={onCancelQuote} type="button">
            <X className="size-3.5" />
          </button>
        </div>
      ) : null}
      <div
        className={cn(
          'overflow-hidden rounded-md border border-input bg-background transition-colors',
          dragging && 'border-ring ring-3 ring-ring/30',
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
          className="min-h-28 w-full resize-y bg-transparent px-3 py-2 text-sm leading-6 outline-none placeholder:text-muted-foreground disabled:cursor-wait disabled:opacity-70"
          disabled={pending}
          id={editorId}
          onChange={(event) => setRequestAttachmentDraftText(draftKey, event.target.value)}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
          placeholder={placeholder}
          ref={textareaRef}
          value={draft.text}
        />
        {draft.attachments.length > 0 ? (
          <div className="px-3 pb-2">
            {draft.attachments.map((attachment) => (
              <DraftAttachmentRow
                attachment={attachment}
                key={attachment.localId}
                onRemove={() => removeUploadingRequestAttachment(draftKey, attachment.localId)}
                onRetry={() => void uploadRequestAttachment({
                  actions: environment.actions,
                  draftKey,
                  localId: attachment.localId,
                  params: environment.params,
                  target: attachmentTargetForDraft(target),
                  onCompleted: () => refreshRequestAttachments(environment.accessScope, environment.requestId),
                })}
                onReselect={() => fileInputRef.current?.click()}
              />
            ))}
          </div>
        ) : null}
        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border bg-muted/25 px-3 py-2">
          <input
            accept={acceptedMedia.join(',')}
            aria-label="Attach photos or videos"
            className="sr-only"
            multiple
            onChange={(event) => {
              addFiles([...(event.target.files ?? [])])
              event.target.value = ''
            }}
            ref={fileInputRef}
            type="file"
          />
          <Button onClick={() => fileInputRef.current?.click()} size="sm" type="button" variant="ghost">
            <Paperclip className="size-3.5" />
            Attach files
          </Button>
          <span className="text-xs text-muted-foreground">Drop files or paste an image</span>
        </div>
      </div>
      {validationError ? <p className="mt-2 text-sm text-destructive" role="alert">{validationError}</p> : null}
      <div className="mt-2 flex items-center justify-between gap-3">
        <p aria-live="polite" className="text-xs text-muted-foreground">
          {attachmentCount > attachmentLimit
            ? `You can attach up to ${attachmentLimit} files here.`
            : hasFailedTransfer
            ? 'Remove or retry failed files before saving.'
            : transferPending
              ? 'Uploading files… Draft kept while you navigate.'
              : enterSubmits
                ? 'Markdown · Shift+Enter for a new line'
                : 'Markdown · Draft kept while you navigate'}
        </p>
        <div className="flex items-center gap-2">
          <Button disabled={pending} onClick={onCancel} size="sm" type="button" variant="ghost">Cancel</Button>
          <Button disabled={!canSubmit} size="sm" type="submit">
            {submitIcon}
            {pending ? 'Saving…' : submitLabel}
          </Button>
        </div>
      </div>
    </form>
  )
}

function DraftAttachmentRow({
  attachment,
  onRemove,
  onReselect,
  onRetry,
}: {
  attachment: DraftAttachment
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
            className="mt-1 block h-1 w-full max-w-60 appearance-none overflow-hidden rounded-full bg-muted [&::-moz-progress-bar]:bg-brand [&::-webkit-progress-bar]:bg-muted [&::-webkit-progress-value]:bg-brand"
            max={1}
            value={attachment.progress}
          />
        ) : null}
      </div>
      <div className="flex items-center">
        {attachment.status === 'failed' ? (
          <Button
            aria-label={attachment.file ? `Retry ${attachment.name}` : `Select ${attachment.name} again`}
            onClick={attachment.file ? onRetry : onReselect}
            size="icon-sm"
            title={attachment.file ? 'Retry upload' : 'Select the same file to resume'}
            type="button"
            variant="ghost"
          ><RotateCcw /></Button>
        ) : null}
        <Button aria-label={`Remove ${attachment.name}`} onClick={onRemove} size="icon-sm" title="Remove attachment" type="button" variant="ghost"><X /></Button>
      </div>
    </div>
  )
}

function markdownWithAttachments(text: string, attachments: DraftAttachment[]) {
  const trimmed = text.trim()
  const references = attachments.flatMap((attachment) => {
    if (!attachment.attachmentId) return []
    const path = requestAttachmentMarkdownReference(attachment.attachmentId)
    if (trimmed.includes(`](${path})`)) return []
    const label = attachment.name.replaceAll('\\', '\\\\').replaceAll(']', '\\]')
    return [attachment.contentType.startsWith('image/')
      ? `![${label}](${path})`
      : `[${label}](${path})`]
  })
  return [trimmed, ...references].filter(Boolean).join('\n\n')
}

function validateFiles(
  files: File[],
  currentCount: number,
  limits: ReturnType<typeof useRequestAttachments>['limits'],
) {
  const available = Math.max(0, (limits?.max_attachments_per_content ?? 10) - currentCount)
  const accepted: File[] = []
  let error: string | null = files.length > available
    ? `You can attach up to ${limits?.max_attachments_per_content ?? 10} files here.`
    : null
  const photoTypes = new Set(limits?.accepted_photo_media_types ??
    FALLBACK_ACCEPTED_MEDIA.filter((type) => type.startsWith('image/')))
  const videoTypes = new Set(limits?.accepted_video_media_types ??
    FALLBACK_ACCEPTED_MEDIA.filter((type) => type.startsWith('video/')))
  for (const file of files.slice(0, available)) {
    const mediaType = browserMediaType(file)
    const isPhoto = photoTypes.has(mediaType)
    const isVideo = videoTypes.has(mediaType)
    if (!isPhoto && !isVideo) {
      error = `${file.name} is not a supported photo or video.`
      continue
    }
    const maxBytes = isVideo ? limits?.max_video_bytes : limits?.max_photo_bytes
    if (maxBytes !== undefined && file.size > maxBytes) {
      error = `${file.name} is larger than the ${formatBytes(maxBytes)} limit.`
      continue
    }
    accepted.push(file)
  }
  return { error, files: accepted }
}

function browserMediaType(file: File) {
  if (file.type) return file.type
  const extension = file.name.split('.').at(-1)?.toLowerCase()
  if (extension === 'heic' || extension === 'heif') return `image/${extension}`
  if (extension === 'mov') return 'video/quicktime'
  if (extension === 'mp4' || extension === 'webm') return `video/${extension}`
  if (extension && ['png', 'jpeg', 'webp', 'gif'].includes(extension)) return `image/${extension}`
  if (extension === 'jpg') return 'image/jpeg'
  return 'application/octet-stream'
}

function formatBytes(bytes: number) {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`
  return `${(bytes / (1024 * 1024)).toFixed(bytes < 10 * 1024 * 1024 ? 1 : 0)} MB`
}
