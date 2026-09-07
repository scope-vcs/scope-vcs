const ATTACHMENT_PATH = /^\/request-attachments\/([^/?#]+)$/

export function requestAttachmentIdFromUrl(url: string | undefined) {
  if (!url) return null
  const match = ATTACHMENT_PATH.exec(url)
  if (!match?.[1]) return null
  try {
    const id = decodeURIComponent(match[1])
    return id && !/[\/\s\u0000-\u001f\u007f]/u.test(id) ? id : null
  } catch {
    return null
  }
}

export function requestAttachmentDraftReference(attachment: {
  contentType: string
  localId: string
  name: string
}) {
  const path = `/request-attachments/${encodeURIComponent(attachment.localId)}`
  const label = attachment.name.replaceAll('\\', '\\\\').replaceAll(']', '\\]')
  return attachment.contentType.startsWith('image/')
    ? `![${label}](${path})`
    : `[${label}](${path})`
}

export function insertRequestAttachmentReferences(
  text: string,
  cursor: number,
  references: string[],
) {
  const boundedCursor = Math.min(Math.max(0, cursor), text.length)
  const before = text.slice(0, boundedCursor)
  const after = text.slice(boundedCursor)
  const insertion = [
    before && !before.endsWith('\n') ? '\n\n' : '',
    references.join('\n\n'),
    after && !after.startsWith('\n') ? '\n\n' : '',
  ].join('')
  return before + insertion + after
}
