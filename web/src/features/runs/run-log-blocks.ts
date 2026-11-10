const LOG_BLOCK_LINES = 200

export function logBlocks<Line>(lines: readonly Line[]): Line[][] {
  const blocks: Line[][] = []
  for (let start = 0; start < lines.length; start += LOG_BLOCK_LINES) {
    blocks.push(lines.slice(start, start + LOG_BLOCK_LINES))
  }
  return blocks
}
