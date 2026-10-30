import { DayPicker } from '@daypicker/react'
import { ChevronLeft, ChevronRight } from 'lucide-react'
import type { ComponentProps } from 'react'

const CELL_CLASS =
  "relative flex size-(--cell-size) items-center justify-center rounded-md text-sm text-foreground not-in-data-selected:hover:bg-muted disabled:pointer-events-none disabled:opacity-45 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0"

const CLASS_NAMES = {
  button_next: CELL_CLASS,
  button_previous: CELL_CLASS,
  caption_label: 'flex h-full items-center text-sm font-medium',
  day: 'size-(--cell-size) py-px text-sm',
  day_button: `${CELL_CLASS} outline-none in-data-disabled:pointer-events-none in-data-disabled:text-muted-foreground/70 in-data-disabled:line-through in-data-outside:text-muted-foreground/70 in-data-selected:bg-primary in-data-selected:text-primary-foreground focus-visible:z-1 focus-visible:outline-2 focus-visible:outline-ring`,
  hidden: 'invisible',
  month: 'w-full',
  month_caption: 'relative z-2 mx-(--cell-size) mb-1 flex h-(--cell-size) items-center justify-center px-1',
  months: 'relative flex flex-col gap-2',
  nav: 'absolute top-0 z-1 flex w-full justify-between',
  outside: 'text-muted-foreground',
  today:
    '*:after:pointer-events-none *:after:absolute *:after:bottom-1 *:after:start-1/2 *:after:z-1 *:after:size-[3px] *:after:-translate-x-1/2 *:after:rounded-full *:after:bg-primary [&[data-disabled]>*]:after:bg-foreground/30 [&[data-selected]>*]:after:bg-background',
  weekday: 'size-(--cell-size) p-0 text-xs font-medium text-muted-foreground/70',
}

function CalendarChevron({ orientation }: { orientation?: 'left' | 'right' | 'up' | 'down' }) {
  return orientation === 'left' ? <ChevronLeft aria-hidden="true" /> : <ChevronRight aria-hidden="true" />
}

export function Calendar(props: ComponentProps<typeof DayPicker>) {
  return (
    <DayPicker
      className="w-fit [--cell-size:--spacing(8)] sm:[--cell-size:--spacing(9)]"
      classNames={CLASS_NAMES}
      components={{ Chevron: CalendarChevron }}
      showOutsideDays
      {...props}
    />
  )
}
