import { useEffect, useState, type RefObject } from 'react'

const DETAIL_RAIL_MIN_WIDTH = 1040

export function useDetailPaneRail(pane: RefObject<HTMLElement | null>) {
  const [rail, setRail] = useState(false)
  useEffect(() => {
    const element = pane.current
    if (!element) return
    const observer = new ResizeObserver(([entry]) => {
      setRail(entry.contentRect.width >= DETAIL_RAIL_MIN_WIDTH)
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [pane])
  return rail
}
