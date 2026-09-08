import { fromMarkdown } from 'mdast-util-from-markdown'

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

type MarkdownNode = {
  type: string
  url?: string
  identifier?: string
  children?: MarkdownNode[]
}

export function requestAttachmentContentCount(
  text: string,
  attachments: ReadonlyArray<{ localId: string; attachmentId: string | null }>,
) {
  const root = fromMarkdown(text)
  const definitions = new Map<string, string>()
  const ids = new Set<string>()
  function visit(node: MarkdownNode, read: (node: MarkdownNode) => void) {
    read(node)
    for (const child of node.children ?? []) visit(child, read)
  }
  visit(root, (node) => {
    if (node.type === 'definition' && node.identifier && node.url && !definitions.has(node.identifier.toUpperCase())) {
      definitions.set(node.identifier.toUpperCase(), node.url)
    }
  })
  visit(root, (node) => {
    const url = node.type === 'link' || node.type === 'image'
      ? node.url
      : (node.type === 'linkReference' || node.type === 'imageReference') && node.identifier
        ? definitions.get(node.identifier.toUpperCase())
        : undefined
    const id = requestAttachmentIdFromUrl(url)
    if (id) ids.add(id)
  })
  for (const attachment of attachments) ids.add(attachment.attachmentId ?? attachment.localId)
  return ids.size
}
