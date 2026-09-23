import { useEffect, useState } from 'react'

/** The element's border-box height, kept current as it resizes; 0 without one. */
export function useElementHeight(element: HTMLElement | null) {
  const [height, setHeight] = useState(0)
  useEffect(() => {
    if (!element) return
    const observer = new ResizeObserver(([entry]) => {
      setHeight(entry.borderBoxSize[0]?.blockSize ?? entry.target.getBoundingClientRect().height)
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [element])
  return element ? height : 0
}
