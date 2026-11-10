import { cn } from '@/lib/utils'
import { useMemo, type ReactNode } from 'react'
import { logBlocks } from './run-log-blocks'

const LOG_LINE_HEIGHT_PX = 20

export function RunLogText({ empty, text, wrap }: { empty: ReactNode; text: string; wrap: boolean }) {
  const blocks = useMemo(() => logBlocks(text), [text])
  return (
    <pre className={cn('mt-1 overflow-x-auto break-words pb-4', wrap ? 'whitespace-pre-wrap' : 'whitespace-pre')}>
      {blocks.length === 0 ? <span className="text-muted-foreground">{empty}</span> : blocks.map((lines, index) => (
        <span
          className={cn('block [content-visibility:auto]', !wrap && 'w-max min-w-full')}
          data-log-block=""
          key={index}
          style={{ containIntrinsicBlockSize: `auto ${lines.length * LOG_LINE_HEIGHT_PX}px` }}
        >
          <span>{lines.join('\n')}</span>
        </span>
      ))}
    </pre>
  )
}
