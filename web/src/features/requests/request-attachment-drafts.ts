export type RequestAttachmentDraftTarget =
  | 'description'
  | 'discussion'
  | `reply:${string}`

export type RequestAttachmentDraftScope = {
  accessScope: string
  repoId: string
  requestId: string
  target: RequestAttachmentDraftTarget
  viewerId: string
}

export type DraftAttachmentStatus =
  | 'queued'
  | 'uploading'
  | 'uploaded'
  | 'failed'

export type DraftAttachment = {
  attachmentId: string | null
  contentType: string
  error: string | null
  file: File | null
  localId: string
  name: string
  operationId: string
  previewUrl: string | null
  progress: number
  sha256: string | null
  size: number
  status: DraftAttachmentStatus
}

export type RequestAttachmentDraft = {
  attachments: DraftAttachment[]
  baseText: string | null
  initialized: boolean
  text: string
}

type Entry = {
  draft: RequestAttachmentDraft
  listeners: Set<() => void>
  touchedAt: number
}

const MAX_DRAFTS = 16
const MAX_RETAINED_FILE_BYTES = 1024 * 1024 * 1024
const STORAGE_PREFIX = 'scope.request-attachment-draft.'
const entries = new Map<string, Entry>()
let cancelUploadsForDraft: (key: string) => void = () => {}

export function requestAttachmentDraftKey(scope: RequestAttachmentDraftScope) {
  return JSON.stringify([
    scope.viewerId,
    scope.repoId,
    scope.requestId,
    scope.accessScope,
    scope.target,
  ])
}

export function readRequestAttachmentDraft(key: string) {
  return entryFor(key).draft
}

export function subscribeRequestAttachmentDraft(
  key: string,
  listener: () => void,
) {
  const entry = entryFor(key)
  entry.listeners.add(listener)
  return () => entry.listeners.delete(listener)
}

export function setRequestAttachmentDraftText(key: string, text: string) {
  update(key, (draft) => draft.text === text && draft.initialized
    ? draft
    : { ...draft, initialized: true, text })
}

export function seedRequestAttachmentDraft(key: string, text: string) {
  update(key, (draft) => draft.initialized
    ? draft
    : { ...draft, baseText: text, initialized: true, text })
}

export function replaceRequestAttachmentDraftReference(
  key: string,
  localId: string,
  attachmentId: string,
) {
  const localPath = requestAttachmentMarkdownReference(localId)
  const finalPath = requestAttachmentMarkdownReference(attachmentId)
  update(key, (draft) => ({
    ...draft,
    text: draft.text.replaceAll(`](${localPath})`, `](${finalPath})`),
  }))
}

export function addRequestAttachmentDraftFiles(key: string, files: File[]) {
  if (files.length === 0) return []
  const entry = entryFor(key)
  const additions: DraftAttachment[] = []
  let attachments = [...entry.draft.attachments]
  for (const file of files) {
    const resumableIndex = attachments.findIndex((attachment) =>
      attachment.file === null &&
      attachment.name === (file.name || 'Pasted image') &&
      attachment.size === file.size,
    )
    if (resumableIndex >= 0) {
      const resumable = attachments[resumableIndex]
      if (!resumable) continue
      const resumed: DraftAttachment = {
        ...resumable,
        contentType: inferredMediaType(file),
        error: null,
        file,
        previewUrl: createPreviewUrl(file),
        progress: 0,
        status: 'queued',
      }
      attachments[resumableIndex] = resumed
      additions.push(resumed)
      continue
    }
    const addition: DraftAttachment = {
      attachmentId: null,
      contentType: inferredMediaType(file),
      error: null,
      file,
      localId: crypto.randomUUID(),
      name: file.name || 'Pasted image',
      operationId: crypto.randomUUID(),
      previewUrl: createPreviewUrl(file),
      progress: 0,
      sha256: null,
      size: file.size,
      status: 'queued',
    }
    attachments.push(addition)
    additions.push(addition)
  }
  update(key, (draft) => ({
    ...draft,
    attachments,
  }))
  evictIdleDrafts()
  return additions
}

export function patchRequestAttachmentDraftFile(
  key: string,
  localId: string,
  patch: Partial<Omit<DraftAttachment, 'file' | 'localId'>>,
) {
  update(key, (draft) => ({
    ...draft,
    attachments: draft.attachments.map((attachment) =>
      attachment.localId === localId
        ? { ...attachment, ...patch }
        : attachment,
    ),
  }))
}

export function removeRequestAttachmentDraftFile(key: string, localId: string) {
  const removed = entryFor(key).draft.attachments.find(
    (attachment) => attachment.localId === localId,
  )
  revokePreviewUrl(removed?.previewUrl)
  update(key, (draft) => ({
    ...draft,
    attachments: draft.attachments.filter(
      (attachment) => attachment.localId !== localId,
    ),
    text: removeReference(
      removeReference(draft.text, localId),
      removed?.attachmentId ?? localId,
    ),
  }))
}

function removeReference(text: string, attachmentId: string) {
  const path = requestAttachmentMarkdownReference(attachmentId)
    .replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  return text
    .replace(new RegExp(`!?\\[[^\\]]*\\]\\(${path}\\)`, 'g'), '')
    .replace(/\n{3,}/g, '\n\n')
    .trim()
}

export function clearRequestAttachmentDraft(key: string) {
  for (const attachment of entryFor(key).draft.attachments) {
    revokePreviewUrl(attachment.previewUrl)
  }
  update(key, () => emptyDraft())
}

export function resetRequestAttachmentDraftManager() {
  for (const entry of entries.values()) {
    for (const attachment of entry.draft.attachments) {
      revokePreviewUrl(attachment.previewUrl)
    }
  }
  entries.clear()
}

export function registerRequestAttachmentDraftUploadCanceller(
  cancel: (key: string) => void,
) {
  cancelUploadsForDraft = cancel
}

export function activateRequestAttachmentDraftScope({
  accessScope,
  repoId,
  viewerId,
}: Pick<RequestAttachmentDraftScope, 'accessScope' | 'repoId' | 'viewerId'>) {
  for (const key of knownDraftKeys()) {
    const parsed = parseDraftKey(key)
    if (!parsed) continue
    const outsideViewer = parsed.viewerId !== viewerId
    const changedRepoAccess = parsed.repoId === repoId && parsed.accessScope !== accessScope
    if (outsideViewer || changedRepoAccess) discardDraftKey(key)
  }
}

export function activateRequestAttachmentDraftViewer(viewerId: string) {
  for (const key of knownDraftKeys()) {
    const parsed = parseDraftKey(key)
    if (parsed && parsed.viewerId !== viewerId) discardDraftKey(key)
  }
}

function knownDraftKeys() {
  const keys = new Set(entries.keys())
  if (typeof sessionStorage !== 'undefined') {
    try {
      for (let index = 0; index < sessionStorage.length; index += 1) {
        const storageKey = sessionStorage.key(index)
        if (storageKey?.startsWith(STORAGE_PREFIX)) keys.add(storageKey.slice(STORAGE_PREFIX.length))
      }
    } catch {
      // In-memory scope cleanup still applies when browser storage is unavailable.
    }
  }
  return keys
}

function discardDraftKey(key: string) {
  cancelUploadsForDraft(key)
  const entry = entries.get(key)
  for (const attachment of entry?.draft.attachments ?? []) revokePreviewUrl(attachment.previewUrl)
  entries.delete(key)
  try {
    sessionStorage.removeItem(`${STORAGE_PREFIX}${key}`)
  } catch {
    // The in-memory value and object URLs have already been cleared.
  }
}

export function requestAttachmentMarkdownReference(attachmentId: string) {
  return `/request-attachments/${encodeURIComponent(attachmentId)}`
}

function entryFor(key: string) {
  const current = entries.get(key)
  if (current) {
    current.touchedAt = Date.now()
    return current
  }
  const created: Entry = {
    draft: restore(key),
    listeners: new Set(),
    touchedAt: Date.now(),
  }
  entries.set(key, created)
  evictIdleDrafts()
  return created
}

function update(
  key: string,
  updater: (draft: RequestAttachmentDraft) => RequestAttachmentDraft,
) {
  const entry = entryFor(key)
  const next = updater(entry.draft)
  if (next === entry.draft) return
  entry.draft = next
  entry.touchedAt = Date.now()
  for (const listener of entry.listeners) listener()
  persist(key, next)
}

function evictIdleDrafts() {
  const retainedBytes = () => [...entries.values()].reduce(
    (total, entry) => total + entry.draft.attachments.reduce(
      (draftTotal, attachment) => draftTotal + (attachment.file?.size ?? 0),
      0,
    ),
    0,
  )
  if (entries.size <= MAX_DRAFTS && retainedBytes() <= MAX_RETAINED_FILE_BYTES) return
  const candidates = [...entries]
    .filter(([, entry]) =>
      entry.listeners.size === 0 &&
      !entry.draft.attachments.some(({ status }) => status === 'uploading'),
    )
    .sort((left, right) => left[1].touchedAt - right[1].touchedAt)
  for (const [key] of candidates) {
    if (entries.size <= MAX_DRAFTS && retainedBytes() <= MAX_RETAINED_FILE_BYTES) break
    discardDraftKey(key)
  }
}

function emptyDraft(): RequestAttachmentDraft {
  return { attachments: [], baseText: null, initialized: false, text: '' }
}

function parseDraftKey(key: string) {
  try {
    const value: unknown = JSON.parse(key)
    if (!Array.isArray(value) || value.length !== 5) return null
    const [viewerId, repoId, requestId, accessScope, target] = value
    if ([viewerId, repoId, requestId, accessScope, target].some((part) => typeof part !== 'string')) return null
    return { accessScope, repoId, requestId, target, viewerId } as RequestAttachmentDraftScope
  } catch {
    return null
  }
}

function inferredMediaType(file: File) {
  if (file.type) return file.type
  const extension = file.name.split('.').at(-1)?.toLowerCase()
  if (extension === 'heic') return 'image/heic'
  if (extension === 'heif') return 'image/heif'
  if (extension === 'mov') return 'video/quicktime'
  if (extension === 'mp4') return 'video/mp4'
  if (extension === 'webm') return 'video/webm'
  if (extension === 'png') return 'image/png'
  if (extension === 'jpg' || extension === 'jpeg') return 'image/jpeg'
  if (extension === 'webp') return 'image/webp'
  if (extension === 'gif') return 'image/gif'
  return 'application/octet-stream'
}

function persist(key: string, draft: RequestAttachmentDraft) {
  if (typeof sessionStorage === 'undefined') return
  try {
    if (!draft.initialized && !draft.text && draft.attachments.length === 0) {
      sessionStorage.removeItem(`${STORAGE_PREFIX}${key}`)
      return
    }
    sessionStorage.setItem(`${STORAGE_PREFIX}${key}`, JSON.stringify({
      attachments: draft.attachments.map((attachment) => ({
        attachmentId: attachment.attachmentId,
        contentType: attachment.contentType,
        error: attachment.attachmentId ? null : 'Select the same file to resume this upload.',
        localId: attachment.localId,
        name: attachment.name,
        operationId: attachment.operationId,
        progress: attachment.attachmentId ? 1 : 0,
        sha256: attachment.sha256,
        size: attachment.size,
        status: attachment.attachmentId ? 'uploaded' : 'failed',
      })),
      baseText: draft.baseText,
      initialized: draft.initialized,
      text: draft.text,
    }))
  } catch {
    // In-memory drafts continue to work when browser storage is unavailable.
  }
}

function restore(key: string): RequestAttachmentDraft {
  if (typeof sessionStorage === 'undefined') return emptyDraft()
  try {
    const raw = sessionStorage.getItem(`${STORAGE_PREFIX}${key}`)
    if (!raw) return emptyDraft()
    const value: unknown = JSON.parse(raw)
    if (!value || typeof value !== 'object') return emptyDraft()
    const record = value as { attachments?: unknown; baseText?: unknown; initialized?: unknown; text?: unknown }
    const attachments = Array.isArray(record.attachments)
      ? record.attachments.flatMap((item): DraftAttachment[] => {
          if (!item || typeof item !== 'object') return []
          const attachment = item as Record<string, unknown>
          if (
            typeof attachment.localId !== 'string' ||
            typeof attachment.name !== 'string' ||
            typeof attachment.operationId !== 'string' ||
            typeof attachment.size !== 'number' ||
            typeof attachment.contentType !== 'string'
          ) return []
          const attachmentId = typeof attachment.attachmentId === 'string'
            ? attachment.attachmentId
            : null
          return [{
            attachmentId,
            contentType: attachment.contentType,
            error: attachmentId ? null : 'Select the same file to resume this upload.',
            file: null,
            localId: attachment.localId,
            name: attachment.name,
            operationId: attachment.operationId,
            previewUrl: null,
            progress: attachmentId ? 1 : 0,
            sha256: typeof attachment.sha256 === 'string' ? attachment.sha256 : null,
            size: attachment.size,
            status: attachmentId ? 'uploaded' : 'failed',
          }]
        })
      : []
    return {
      attachments,
      baseText: typeof record.baseText === 'string' ? record.baseText : null,
      initialized: record.initialized === true,
      text: typeof record.text === 'string' ? record.text : '',
    }
  } catch {
    return emptyDraft()
  }
}

function createPreviewUrl(file: File) {
  return ['image/png', 'image/jpeg', 'image/webp', 'image/gif'].includes(inferredMediaType(file)) &&
    typeof URL.createObjectURL === 'function'
    ? URL.createObjectURL(file)
    : null
}

function revokePreviewUrl(url: string | null | undefined) {
  if (url && typeof URL.revokeObjectURL === 'function') URL.revokeObjectURL(url)
}
