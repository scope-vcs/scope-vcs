const LOG_BLOCK_LINES = 200

export type LogBlock<Line> = { start: number; lines: Line[] }

export function logBlocks<Line>(lines: readonly Line[]): LogBlock<Line>[] {
  const blocks: LogBlock<Line>[] = []
  for (let start = 0; start < lines.length; start += LOG_BLOCK_LINES) {
    blocks.push({ start, lines: lines.slice(start, start + LOG_BLOCK_LINES) })
  }
  return blocks
}
