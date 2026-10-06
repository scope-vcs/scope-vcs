import { useLayoutEffect, useRef } from 'react'

export function useRunJobHash(select: (hash: string) => boolean) {
  const selectRef = useRef(select)
  useLayoutEffect(() => {
    selectRef.current = select
  })
  useLayoutEffect(() => {
    let frame = 0
    const onHash = () => {
      const hash = window.location.hash
      if (!selectRef.current(hash)) return
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(() => {
        document
          .querySelector(`button[aria-controls="${CSS.escape(hash.slice(1))}"]`)
          ?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
      })
    }
    onHash()
    window.addEventListener('hashchange', onHash)
    return () => {
      cancelAnimationFrame(frame)
      window.removeEventListener('hashchange', onHash)
    }
  }, [])
}
