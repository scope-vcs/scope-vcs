import { useLayoutEffect, useRef } from 'react'
import { runJobPanelId } from './run-job-ids'

export function useRunJobHash(select: (hash: string) => string | null) {
  const selectRef = useRef(select)
  useLayoutEffect(() => {
    selectRef.current = select
  })
  useLayoutEffect(() => {
    let frame = 0
    const onHash = () => {
      const key = selectRef.current(window.location.hash)
      if (key === null) return
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(() => {
        document
          .querySelector(`button[aria-controls="${CSS.escape(runJobPanelId(key))}"]`)
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
