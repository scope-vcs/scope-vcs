import { useLayoutEffect, useState } from 'react'

export function useElementHeight(element: HTMLElement | null) {
  const [height, setHeight] = useState(0)
  useLayoutEffect(() => {
    if (!element) return
    setHeight(element.getBoundingClientRect().height)
    const observer = new ResizeObserver(([entry]) => {
      setHeight(entry.borderBoxSize[0]?.blockSize ?? entry.target.getBoundingClientRect().height)
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [element])
  return element ? height : 0
}
