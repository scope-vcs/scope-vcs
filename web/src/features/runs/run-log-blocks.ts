const LOG_BLOCK_LINES = 200

export type LogBlock = { text: string; lineCount: number }

export function logBlocks(text: string): LogBlock[] {
  const lines = text.split('\n')
  if (lines.at(-1) === '') lines.pop()
  const blocks: LogBlock[] = []
  for (let start = 0; start < lines.length; start += LOG_BLOCK_LINES) {
    const blockLines = lines.slice(start, start + LOG_BLOCK_LINES)
    const last = start + LOG_BLOCK_LINES >= lines.length
    blocks.push({ text: blockLines.join('\n') + (last ? '' : '\n'), lineCount: blockLines.length })
  }
  return blocks
}
