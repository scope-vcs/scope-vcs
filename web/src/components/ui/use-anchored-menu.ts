import { useEffect, useId, useRef, useState, type KeyboardEvent, type ToggleEvent } from 'react'

export const ANCHORED_MENU_CLASS = 'fixed m-0 max-w-[calc(100vw-16px)] overflow-y-auto rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-[var(--shadow-pop)]'

const GAP = 6
const EDGE = 8

export function useAnchoredMenu({
  align,
  onOpenChange,
}: {
  align: 'start' | 'end'
  onOpenChange?: (open: boolean) => void
}) {
  const id = useId()
  const trigger = useRef<HTMLButtonElement>(null)
  const menu = useRef<HTMLDivElement>(null)
  const [open, setOpen] = useState(false)

  useEffect(() => {
    if (!open) return
    const hide = (event: Event) => {
      if (event.target instanceof Node && menu.current?.contains(event.target)) return
      menu.current?.hidePopover()
    }
    window.addEventListener('resize', hide)
    window.addEventListener('scroll', hide, true)
    return () => {
      window.removeEventListener('resize', hide)
      window.removeEventListener('scroll', hide, true)
    }
  }, [open])

  function toggle(event: ToggleEvent<HTMLDivElement>) {
    const showing = event.newState === 'open'
    setOpen(showing)
    onOpenChange?.(showing)
    if (showing && trigger.current && menu.current) placeMenu(trigger.current, menu.current, align)
  }

  return {
    hide: () => menu.current?.hidePopover(),
    menuProps: {
      id,
      onKeyDown: moveFocus,
      onToggle: toggle,
      popover: 'auto' as const,
      ref: menu,
      role: 'menu',
      tabIndex: -1,
    },
    open,
    triggerProps: {
      'aria-expanded': open,
      'aria-haspopup': 'menu' as const,
      popoverTarget: id,
      ref: trigger,
    },
  }
}

function placeMenu(trigger: HTMLElement, menu: HTMLElement, align: 'start' | 'end') {
  const bounds = trigger.getBoundingClientRect()
  menu.style.maxHeight = ''
  const below = window.innerHeight - bounds.bottom - GAP - EDGE
  const above = bounds.top - GAP - EDGE
  const placeBelow = menu.scrollHeight <= below || below >= above
  const maxHeight = placeBelow ? below : above
  menu.style.maxHeight = `${maxHeight}px`
  const height = Math.min(menu.offsetHeight, maxHeight)
  const width = menu.offsetWidth
  const preferredLeft = align === 'start' ? bounds.left : bounds.right - width
  menu.style.top = `${placeBelow ? bounds.bottom + GAP : bounds.top - height - GAP}px`
  menu.style.left = `${Math.max(EDGE, Math.min(preferredLeft, window.innerWidth - width - EDGE))}px`
  const initial = menu.querySelector<HTMLElement>('[aria-checked="true"]') ?? menu.querySelector<HTMLElement>('button')
  focusItem(menu, initial)
}

function moveFocus(event: KeyboardEvent<HTMLDivElement>) {
  const buttons = Array.from(event.currentTarget.querySelectorAll('button'))
  const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
  const next = {
    Home: 0,
    End: buttons.length - 1,
    ArrowDown: (index + 1) % buttons.length,
    ArrowUp: (index - 1 + buttons.length) % buttons.length,
  }[event.key]
  if (next === undefined) return
  event.preventDefault()
  focusItem(event.currentTarget, buttons[next])
}

function focusItem(menu: HTMLElement, item: HTMLElement | null | undefined) {
  if (!item) return
  item.focus({ preventScroll: true })
  const { top, bottom } = item.getBoundingClientRect()
  const visibleTop = menu.getBoundingClientRect().top + menu.clientTop
  const visibleBottom = visibleTop + menu.clientHeight
  menu.scrollTop += top < visibleTop ? top - visibleTop : Math.max(0, bottom - visibleBottom)
}
