import { useEffect, useState, type RefObject } from 'react'

export function useRequestDetailsRailVisibility(
  pane: RefObject<HTMLElement | null>,
  trigger: RefObject<HTMLElement | null>,
) {
  const [visible, setVisible] = useState(false)
  useEffect(() => {
    const element = pane.current
    const button = trigger.current
    if (!element || !button) return
    const update = () => setVisible(getComputedStyle(button).display === 'none')
    update()
    const observer = new ResizeObserver(update)
    observer.observe(element)
    return () => observer.disconnect()
  }, [pane, trigger])
  return visible
}
