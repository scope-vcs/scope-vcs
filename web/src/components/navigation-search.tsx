import { LoaderCircle, Search, X } from 'lucide-react'
import { useEffect, useRef, type KeyboardEventHandler } from 'react'
import { cn } from '@/lib/utils'

export function NavigationSearch({
  label,
  placeholder,
  clearLabel,
  value,
  onChange,
  onOpen,
  onKeyDown,
  describedBy,
  status,
}: {
  label: string
  placeholder: string
  clearLabel: string
  value: string
  onChange: (value: string) => void
  onOpen: () => void
  onKeyDown?: KeyboardEventHandler<HTMLInputElement>
  describedBy?: string
  status?: string
}) {
  const inputRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    function focusSearch(event: KeyboardEvent) {
      if (
        event.key !== '/' ||
        event.defaultPrevented ||
        event.isComposing ||
        event.metaKey ||
        event.ctrlKey ||
        event.altKey
      )
        return
      if (
        event.target instanceof HTMLElement &&
        event.target.closest(
          'input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"]',
        )
      )
        return
      if (!inputRef.current) return
      event.preventDefault()
      onOpen()
      requestAnimationFrame(() => inputRef.current?.focus())
    }
    document.addEventListener('keydown', focusSearch)
    return () => document.removeEventListener('keydown', focusSearch)
  }, [onOpen])

  function clear() {
    onChange('')
    inputRef.current?.focus()
  }

  return (
    <search className="relative flex min-w-0 flex-1 items-center">
      <Search
        aria-hidden="true"
        className="pointer-events-none absolute left-2 size-3.5 text-muted-foreground"
      />
      <input
        aria-describedby={describedBy}
        aria-label={label}
        autoComplete="off"
        className={cn(
          'h-8 w-full min-w-0 text-ellipsis rounded border border-border bg-background pr-8 pl-7 text-xs placeholder:text-muted-foreground focus-visible:outline-2 focus-visible:outline-ring',
          status && value && 'pr-16',
        )}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.preventDefault()
            clear()
          } else onKeyDown?.(event)
        }}
        placeholder={placeholder}
        ref={inputRef}
        type="search"
        value={value}
      />
      {value ? (
        <button
          aria-label={clearLabel}
          className="absolute right-1 rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
          onClick={clear}
          type="button"
        >
          <X aria-hidden="true" className="size-3.5" />
        </button>
      ) : (
        !status && (
          <kbd
            aria-hidden="true"
            className="pointer-events-none absolute right-2 text-[11px] text-muted-foreground"
          >
            /
          </kbd>
        )
      )}
      {status && (
        <LoaderCircle
          aria-label={status}
          className={cn(
            'absolute size-3.5 animate-spin text-muted-foreground',
            value ? 'right-8' : 'right-2',
          )}
        />
      )}
    </search>
  )
}
