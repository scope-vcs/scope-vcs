import { useEffect, useState, type RefObject } from 'react'
import { lensCovers, type LensFrame } from '../lens/lens-motion'

const CHECK_INTERVAL_MS = 150
const MIN_RADIUS = 60
const FLASH_MS = 2200

export interface NoteProgress {
  found: number
  total: number
  moreBelow: boolean
  justFound: boolean
}

/** Counts the notes the lens has settled over. A note counts once it's on
 * screen and inside the lens; `onAllFound` fires once, the first time every note
 * laid out at this width has been found. */
export function useNoteFinder(page: RefObject<HTMLDivElement | null>, lens: RefObject<LensFrame>, onAllFound: () => void): NoteProgress {
  const [progress, setProgress] = useState<NoteProgress>({ found: 0, total: 0, moreBelow: false, justFound: false })

  useEffect(() => {
    const layer = page.current?.querySelector('[data-view="public"]')
    if (!layer) return
    const found = new Set<Element>()
    let celebrated = false
    let flashTimer = 0

    const check = () => {
      const pageBox = page.current?.getBoundingClientRect()
      if (!pageBox) return
      const notes = [...layer.querySelectorAll('[data-note]')].filter((note) => note.getClientRects().length > 0)
      const { x, y, r } = lens.current
      const viewport = { width: innerWidth, height: innerHeight }
      let foundNow = false
      if (r >= MIN_RADIUS) {
        for (const note of notes) {
          if (found.has(note) || !lensCovers(note.getBoundingClientRect(), x + pageBox.left, y + pageBox.top, r, viewport)) continue
          found.add(note)
          foundNow = true
        }
      }
      const moreBelow = notes.some((note) => !found.has(note) && note.getBoundingClientRect().top > innerHeight)
      if (foundNow) {
        clearTimeout(flashTimer)
        flashTimer = window.setTimeout(() => setProgress((current) => ({ ...current, justFound: false })), FLASH_MS)
      }
      setProgress((current) => current.found === found.size && current.total === notes.length && current.moreBelow === moreBelow && !foundNow
        ? current
        : { found: found.size, total: notes.length, moreBelow, justFound: foundNow || current.justFound })
      if (!celebrated && notes.length > 0 && found.size === notes.length) {
        celebrated = true
        onAllFound()
      }
    }

    const interval = window.setInterval(check, CHECK_INTERVAL_MS)
    return () => {
      clearInterval(interval)
      clearTimeout(flashTimer)
    }
  }, [lens, onAllFound, page])

  return progress
}
