import type {
  RequestAttachmentDerivativeKind,
  RequestAttachmentResponse,
} from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { useCachedResource } from '@/lib/use-cached-resource'
import * as Dialog from '@radix-ui/react-dialog'
import { CircleAlert, Download, Expand, LoaderCircle, RotateCcw, X } from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import { useRequestAttachments } from './request-attachment-context'
import { requestAttachmentResource, requestAttachmentResourceIdentity } from './request-attachment-resource'
import { requestAttachmentMediaGrantResource } from './request-attachment-media-resource'

export function RequestAttachmentMedia({
  attachmentId,
  label,
}: {
  attachmentId: string
  label: string
}) {
  const environment = useRequestAttachments()
  const attachment = environment.attachments.get(attachmentId)
  if (!attachment) {
    return (
      <AttachmentStatus icon={<LoaderCircle className="animate-spin" />}>
        {environment.attachmentsError ?? (environment.attachmentsLoading
          ? `Loading ${label || 'attachment'}…`
          : 'This attachment is unavailable.')}
      </AttachmentStatus>
    )
  }
  if (attachment.state !== 'Ready') {
    return <AttachmentNotReady attachment={attachment} />
  }
  return attachment.kind === 'Photo'
    ? <PhotoAttachment attachment={attachment} label={label} />
    : <VideoAttachment attachment={attachment} label={label} />
}

function PhotoAttachment({
  attachment,
  label,
}: {
  attachment: RequestAttachmentResponse
  label: string
}) {
  const [open, setOpen] = useState(false)
  const derivative = derivativeOf(attachment, 'ImagePreview')
  const media = useAttachmentMediaGrant(attachment.id, derivative?.id)
  const alt = label || attachment.filename
  if (!derivative) return <MissingDerivative attachment={attachment} label="Photo preview" />
  if (media.status !== 'loaded') {
    return <MediaLoadStatus filename={attachment.filename} media={media} />
  }
  return (
    <>
      <figure className="my-3 min-w-0">
        <button
          aria-label={`Enlarge ${alt}`}
          className="group relative block max-w-full overflow-hidden rounded-md border border-border bg-muted/20 focus-visible:outline-2 focus-visible:outline-ring"
          onClick={() => setOpen(true)}
          type="button"
        >
          <img
            alt={alt}
            className="max-h-[38rem] max-w-full object-contain"
            height={derivative.height ?? undefined}
            loading="lazy"
            src={media.value.media_url}
            width={derivative.width ?? undefined}
          />
          <span className="absolute bottom-2 right-2 rounded bg-background/90 p-1.5 opacity-0 shadow transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"><Expand className="size-4" /></span>
        </button>
        <AttachmentCaption attachment={attachment} />
      </figure>
      <Dialog.Root onOpenChange={setOpen} open={open}>
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-50 bg-black/75" />
          <Dialog.Content aria-describedby={undefined} className="fixed inset-4 z-50 grid grid-rows-[auto_minmax(0,1fr)] overflow-hidden rounded-md border border-border bg-background shadow-[var(--shadow-pop)] outline-none sm:inset-8">
            <div className="flex items-center gap-3 border-b border-border px-4 py-3">
              <Dialog.Title className="min-w-0 flex-1 truncate text-sm font-semibold">{attachment.filename}</Dialog.Title>
              <OriginalDownload attachment={attachment} />
              <Dialog.Close asChild><Button aria-label="Close photo viewer" size="icon-sm" type="button" variant="ghost"><X /></Button></Dialog.Close>
            </div>
            <div className="grid min-h-0 place-items-center overflow-auto bg-black/5 p-3">
              <img alt={alt} className="max-h-full max-w-full object-contain" src={media.value.media_url} />
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </>
  )
}

function VideoAttachment({
  attachment,
  label,
}: {
  attachment: RequestAttachmentResponse
  label: string
}) {
  const playback = derivativeOf(attachment, 'VideoPlayback')
  const poster = derivativeOf(attachment, 'VideoPoster')
  const media = useAttachmentMediaGrant(attachment.id, playback?.id)
  const posterMedia = useAttachmentMediaGrant(attachment.id, poster?.id, Boolean(poster))
  const videoRef = useRef<HTMLVideoElement>(null)
  const positionRef = useRef(0)
  const playingRef = useRef(false)
  const source = media.value?.media_url

  useEffect(() => {
    const video = videoRef.current
    if (!video || !source) return
    const position = positionRef.current
    const wasPlaying = playingRef.current
    const restore = () => {
      if (position > 0 && Number.isFinite(video.duration)) {
        video.currentTime = Math.min(position, video.duration)
      }
      if (wasPlaying) void video.play().catch(() => {})
    }
    video.addEventListener('loadedmetadata', restore, { once: true })
    video.load()
    return () => video.removeEventListener('loadedmetadata', restore)
  }, [source])

  if (!playback) return <MissingDerivative attachment={attachment} label="Video playback" />
  if (media.status !== 'loaded') {
    return <MediaLoadStatus filename={attachment.filename} media={media} />
  }
  return (
    <figure className="my-3 min-w-0">
      {/* Uploaded recordings have no caption asset; revisit when caption upload or transcription is supported. */}
      {/* eslint-disable-next-line react-doctor/media-has-caption */}
      <video
        aria-label={label || attachment.filename}
        className="max-h-[38rem] w-full rounded-md border border-border bg-black"
        controls
        onError={media.retry}
        onPause={() => { playingRef.current = false }}
        onPlay={() => { playingRef.current = true }}
        onTimeUpdate={(event) => { positionRef.current = event.currentTarget.currentTime }}
        playsInline
        poster={posterMedia.value?.media_url}
        preload="metadata"
        ref={videoRef}
        src={media.value.media_url}
      />
      <AttachmentCaption attachment={attachment} />
    </figure>
  )
}

function AttachmentNotReady({ attachment }: { attachment: RequestAttachmentResponse }) {
  const environment = useRequestAttachments()
  const [retrying, setRetrying] = useState(false)
  const [retryError, setRetryError] = useState<string | null>(null)
  const failure = attachment.failure
  const canRetry = attachment.state === 'Failed' && failure?.retryable && (
    environment.isMaintainer || environment.viewerId === attachment.uploader_user_id
  )
  function retryProcessing() {
    setRetrying(true)
    setRetryError(null)
    void environment.actions.retry({
      ...environment.params,
      attachment_id: attachment.id,
      operation_id: crypto.randomUUID(),
    }).then((updated) => {
      const identity = requestAttachmentResourceIdentity(environment.accessScope, environment.requestId)
      const current = requestAttachmentResource.peek(identity)
      if (current) requestAttachmentResource.write(identity, {
        ...current,
        attachments: current.attachments.map((value) => value.id === updated.id ? updated : value),
      })
    }).catch((error: unknown) => {
      setRetryError(error instanceof Error ? error.message : 'Processing could not be retried.')
    }).finally(() => setRetrying(false))
  }

  return (
    <AttachmentStatus icon={failure ? <CircleAlert /> : <LoaderCircle className="animate-spin" />}>
      <span className="font-medium text-foreground">{attachment.filename}</span>
      <span>{failure?.message ?? (attachment.state === 'Rejected' ? 'This media was rejected.' : 'Preparing preview…')}</span>
      {canRetry ? (
        <Button
          disabled={retrying}
          onClick={retryProcessing}
          size="sm"
          type="button"
          variant="secondary"
        ><RotateCcw />{retrying ? 'Retrying…' : 'Retry processing'}</Button>
      ) : null}
      {retryError ? <span className="text-destructive" role="alert">{retryError}</span> : null}
      {attachment.original_download_available ? (
        <><OriginalDownload attachment={attachment} /><span className="text-[11px]">Original may retain location and camera metadata.</span></>
      ) : null}
    </AttachmentStatus>
  )
}

function AttachmentCaption({ attachment }: { attachment: RequestAttachmentResponse }) {
  const isGif = attachment.detected_media_type === 'image/gif'
  return (
    <figcaption className="mt-1.5 flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
      <span>{attachment.filename} · {formatBytes(attachment.size_bytes)}{isGif ? ' · Still preview' : ''}</span>
      {attachment.original_download_available ? <OriginalDownload attachment={attachment} /> : null}
      {attachment.original_download_available ? (
        <span className="basis-full text-[11px]">Original downloads may retain location and camera metadata.</span>
      ) : null}
    </figcaption>
  )
}

function OriginalDownload({ attachment }: { attachment: RequestAttachmentResponse }) {
  const environment = useRequestAttachments()
  const [pending, setPending] = useState(false)
  const [error, setError] = useState(false)
  return (
    <Button
      disabled={pending}
      onClick={() => {
        setPending(true)
        setError(false)
        void environment.actions.grant({
          ...environment.params,
          attachment_id: attachment.id,
          target: { kind: 'original' },
        }).then(({ media_url }) => {
          window.location.assign(safeMediaUrl(media_url))
        }).catch(() => setError(true)).finally(() => setPending(false))
      }}
      size="sm"
      type="button"
      variant="ghost"
      title="Original files can retain location and camera metadata."
    ><Download />{pending ? 'Preparing…' : error ? 'Retry download' : 'Download original'}</Button>
  )
}

function AttachmentStatus({ children, icon }: { children: ReactNode; icon: ReactNode }) {
  return <div className="my-3 flex flex-wrap items-center gap-2 border-y border-border py-3 text-sm text-muted-foreground">{icon}<span className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1">{children}</span></div>
}

function MediaLoadStatus({
  filename,
  media,
}: {
  filename: string
  media: ReturnType<typeof useAttachmentMediaGrant>
}) {
  return (
    <AttachmentStatus icon={media.error ? <CircleAlert /> : <LoaderCircle className="animate-spin" />}>
      <span>{media.error ?? `Loading ${filename}…`}</span>
      {media.error ? <Button onClick={media.retry} size="sm" type="button" variant="secondary">Retry</Button> : null}
    </AttachmentStatus>
  )
}

function MissingDerivative({
  attachment,
  label,
}: {
  attachment: RequestAttachmentResponse
  label: string
}) {
  return (
    <AttachmentStatus icon={<CircleAlert />}>
      <span>{label} is unavailable for {attachment.filename}.</span>
      {attachment.original_download_available ? <OriginalDownload attachment={attachment} /> : null}
    </AttachmentStatus>
  )
}

function useAttachmentMediaGrant(
  attachmentId: string,
  derivativeId: string | undefined,
  enabled = true,
) {
  const environment = useRequestAttachments()
  const targetKey = JSON.stringify({ kind: 'derivative', derivative_id: derivativeId })
  const identity = derivativeId
    ? `${environment.accessScope}\0${attachmentId}\0${targetKey}`
    : null
  const load = useCallback(
    () => {
      if (!derivativeId) throw new Error('The media derivative is unavailable.')
      return environment.actions.grant({
        ...environment.params,
        attachment_id: attachmentId,
        target: { kind: 'derivative', derivative_id: derivativeId },
      }).then((grant) => ({ ...grant, media_url: safeMediaUrl(grant.media_url) }))
    }, [attachmentId, derivativeId, environment.actions, environment.params],
  )
  const media = useCachedResource({
    enabled,
    fallbackError: 'The media URL could not be renewed.',
    identity,
    load,
    resource: requestAttachmentMediaGrantResource,
  })
  useEffect(() => {
    if (!identity || !media.value) return
    const delay = Math.max(1_000, media.value.expires_at_unix * 1_000 - Date.now() - 30_000)
    const timeout = window.setTimeout(() => requestAttachmentMediaGrantResource.invalidate(identity), delay)
    return () => window.clearTimeout(timeout)
  }, [identity, media.value])
  return media
}

function derivativeOf(
  attachment: RequestAttachmentResponse,
  kind: RequestAttachmentDerivativeKind,
) {
  return attachment.derivatives.find((derivative) => derivative.kind === kind)
}

function formatBytes(bytes: number) {
  return bytes < 1024 * 1024
    ? `${Math.max(1, Math.round(bytes / 1024))} KB`
    : `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function safeMediaUrl(value: string) {
  const url = new URL(value)
  const local = url.protocol === 'http:' && ['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname)
  if (url.protocol !== 'https:' && !local) throw new Error('The media service returned an unsafe URL.')
  return url.toString()
}
