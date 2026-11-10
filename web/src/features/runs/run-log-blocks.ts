const LOG_BLOCK_LINES = 200

export function logBlocks(text: string): string[][] {
  const lines = text.split('\n')
  if (lines.at(-1) === '') lines.pop()
  const blocks: string[][] = []
  for (let start = 0; start < lines.length; start += LOG_BLOCK_LINES) {
    blocks.push(lines.slice(start, start + LOG_BLOCK_LINES))
  }
  return blocks
}
