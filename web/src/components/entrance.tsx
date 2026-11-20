import { cn } from '@/lib/utils'
import { useCallback, type ComponentProps, type Ref } from 'react'

let entrance: { href: string; element: HTMLElement | null } | null = null

export function enterRef(element: HTMLElement | null) {
  if (!element) return
  const { href } = window.location
  if (entrance === null) {
    entrance = { href, element: null }
    settle(element)
    return
  }
  if (entrance.href !== href) {
    entrance = { href, element }
    settleAfterEntering(element)
    return
  }
  if (entrance.element?.isConnected && element.contains(entrance.element)) {
    settle(entrance.element)
    entrance.element = element
    settleAfterEntering(element)
    return
  }
  settle(element)
}

export function Entrance({ className, ref, ...props }: ComponentProps<'div'>) {
  const attach = useCallback((element: HTMLDivElement | null) => {
    enterRef(element)
    assignRef(ref, element)
  }, [ref])
  return <div className={cn('scope-content-enter', className)} ref={attach} {...props} />
}

function settle(element: HTMLElement) {
  element.style.animation = 'none'
}

function settleAfterEntering(element: HTMLElement) {
  const ended = (event: AnimationEvent) => {
    if (event.target !== element) return
    element.removeEventListener('animationend', ended)
    settle(element)
  }
  element.addEventListener('animationend', ended)
}

function assignRef<T>(ref: Ref<T> | undefined, value: T | null) {
  if (typeof ref === 'function') ref(value)
  else if (ref) ref.current = value
}
