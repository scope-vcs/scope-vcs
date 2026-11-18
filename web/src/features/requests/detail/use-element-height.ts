import { useCallback, useSyncExternalStore } from 'react'

export function useElementHeight(element: HTMLElement | null) {
  const subscribe = useCallback(
    (notify: () => void) => {
      if (!element) return () => {}
      const observer = new ResizeObserver(notify)
      observer.observe(element)
      return () => observer.disconnect()
    },
    [element],
  )
  return useSyncExternalStore(subscribe, () => element?.getBoundingClientRect().height ?? 0, () => 0)
}
