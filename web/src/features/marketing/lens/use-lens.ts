import { useEffect, useRef, useState, type RefObject } from 'react'
import { floodRadius, lensZoom, restingPoint, restRadius, ringOpacity, stepLens, tickPaths, type LensFrame } from './lens-motion'

export interface LensElements {
  page: RefObject<HTMLDivElement | null>
  privateLayer: RefObject<HTMLDivElement | null>
  ring: RefObject<HTMLDivElement | null>
  edge: RefObject<SVGCircleElement | null>
  minorTicks: RefObject<SVGPathElement | null>
  majorTicks: RefObject<SVGPathElement | null>
  ticks: RefObject<SVGGElement | null>
  label: RefObject<SVGTextElement | null>
  grab: RefObject<HTMLDivElement | null>
  cursor: RefObject<HTMLDivElement | null>
  tally: RefObject<HTMLDivElement | null>
}

type Mode = 'rest' | 'follow' | 'drag' | 'pinned'

const OPEN_DELAY_MS = 950
const HINT_DELAY_MS = 3200

/**
 * Drives the lens: follows the mouse, rests over the private rows when there's
 * no pointer, floods the page while the mouse is held, closes over links and
 * buttons, drags on touch, and goes away with the L key. Per-frame work writes
 * straight to the elements; React state only changes for discrete events.
 */
export function useLens(elements: LensElements) {
  const frame = useRef<LensFrame>({ x: 0, y: 0, r: 0, rotation: 0 })
  const [on, setOn] = useState(true)
  const [holding, setHolding] = useState(false)
  const [hint, setHint] = useState(false)
  const [message, setMessage] = useState('')
  const [touch, setTouch] = useState(false)

  useEffect(() => {
    const page = elements.page.current
    const privateLayer = elements.privateLayer.current
    const ring = elements.ring.current
    const grab = elements.grab.current
    if (!page || !privateLayer || !ring || !grab) return

    const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches
    setTouch(matchMedia('(pointer: coarse)').matches)
    const anchor = page.querySelector('[data-view="public"] [data-lens-anchor]')
    const input = { mode: 'rest' as Mode, clientX: 0, clientY: 0, pin: { x: 0, y: 0 }, held: false, over: false, opened: reduced, on: true }
    let drawnRadius = -1
    let animation = 0
    let messageTimer = 0

    const targetRadius = () => {
      if (input.held) return floodRadius(innerWidth, innerHeight)
      return !input.on || !input.opened || input.over ? 0 : restRadius(innerWidth)
    }

    const tick = (time: number) => {
      const pageBox = page.getBoundingClientRect()
      const viewport = { width: innerWidth, height: innerHeight }
      let target = input.mode === 'follow'
        ? { x: input.clientX - pageBox.left, y: input.clientY - pageBox.top }
        : input.mode === 'rest' ? null : input.pin
      if (!target) {
        target = anchor
          ? restingPoint(anchor.getBoundingClientRect(), pageBox, viewport, time, !reduced)
          : { x: viewport.width / 2 - pageBox.left, y: viewport.height / 2 - pageBox.top }
      }
      const next = stepLens(frame.current, { ...target, r: targetRadius() }, { instant: reduced || input.mode === 'drag', resting: input.mode === 'rest' })
      frame.current = next

      const rest = restRadius(viewport.width)
      const zoom = lensZoom(next.r, rest, reduced)
      privateLayer.style.transformOrigin = `${next.x}px ${next.y}px`
      privateLayer.style.transform = `scale(${zoom})`
      privateLayer.style.clipPath = `circle(${Math.max(0, next.r / zoom)}px at ${next.x}px ${next.y}px)`
      ring.style.transform = `translate(${next.x}px, ${next.y}px)`
      ring.style.opacity = String(ringOpacity(next.r, rest))
      if (Math.abs(next.r - drawnRadius) > .4) {
        drawnRadius = next.r
        const { minor, major } = tickPaths(next.r)
        elements.edge.current?.setAttribute('r', String(next.r))
        elements.minorTicks.current?.setAttribute('d', minor)
        elements.majorTicks.current?.setAttribute('d', major)
        elements.label.current?.setAttribute('y', String(next.r - 20))
        grab.style.cssText = `left:${-next.r}px;top:${-next.r}px;width:${2 * next.r}px;height:${2 * next.r}px`
      }
      elements.ticks.current?.setAttribute('transform', `rotate(${next.rotation})`)
      animation = requestAnimationFrame(tick)
    }

    const release = () => {
      if (!input.held) return
      input.held = false
      setHolding(false)
    }

    const onPointerMove = (event: PointerEvent) => {
      if (event.pointerType !== 'mouse') return
      input.mode = 'follow'
      input.clientX = event.clientX
      input.clientY = event.clientY
      const position = `translate(${event.clientX}px, ${event.clientY}px)`
      if (elements.cursor.current) elements.cursor.current.style.transform = position
      if (elements.tally.current) elements.tally.current.style.transform = position
      elements.cursor.current?.classList.add('is-visible')
      const over = event.target instanceof Element && event.target.closest('a, button') !== null
      elements.cursor.current?.classList.toggle('is-over', over)
      input.over = over
    }
    const onPointerLeave = (event: PointerEvent) => {
      if (event.pointerType !== 'mouse') return
      elements.cursor.current?.classList.remove('is-visible')
      if (input.held) return
      input.mode = 'rest'
      input.over = false
    }
    const onPointerDown = (event: PointerEvent) => {
      if (event.pointerType === 'mouse') elements.cursor.current?.classList.add('is-down')
      if (!input.on || event.pointerType !== 'mouse' || event.button !== 0) return
      if (event.target instanceof Element && event.target.closest('a, button')) return
      input.held = true
      setHolding(true)
      setHint(false)
    }
    const onPointerUp = () => {
      elements.cursor.current?.classList.remove('is-down')
      release()
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== 'l' || event.metaKey || event.ctrlKey || event.altKey || event.repeat) return
      if (event.target instanceof HTMLElement && event.target.closest('input, textarea, [contenteditable="true"]')) return
      input.on = !input.on
      release()
      setOn(input.on)
      setMessage(input.on ? 'lens is back' : 'lens away. press L to bring it back')
      clearTimeout(messageTimer)
      messageTimer = window.setTimeout(() => setMessage(''), 2600)
    }
    const onGrabDown = (event: PointerEvent) => {
      grab.setPointerCapture(event.pointerId)
      input.mode = 'drag'
      setHint(false)
    }
    const onGrabMove = (event: PointerEvent) => {
      if (input.mode !== 'drag') return
      const pageBox = page.getBoundingClientRect()
      input.pin = { x: event.clientX - pageBox.left, y: event.clientY - pageBox.top }
    }
    const onGrabUp = () => {
      if (input.mode === 'drag') input.mode = 'pinned'
    }

    const first = anchor ? restingPoint(anchor.getBoundingClientRect(), page.getBoundingClientRect(), { width: innerWidth, height: innerHeight }, 0, false) : null
    if (first) frame.current = { ...frame.current, ...first }
    const openTimer = window.setTimeout(() => { input.opened = true }, reduced ? 0 : OPEN_DELAY_MS)
    const hintTimer = window.setTimeout(() => setHint(true), reduced ? 0 : HINT_DELAY_MS)
    animation = requestAnimationFrame(tick)

    addEventListener('pointermove', onPointerMove)
    document.documentElement.addEventListener('pointerleave', onPointerLeave)
    page.addEventListener('pointerdown', onPointerDown)
    addEventListener('pointerup', onPointerUp)
    addEventListener('keydown', onKeyDown)
    grab.addEventListener('pointerdown', onGrabDown)
    grab.addEventListener('pointermove', onGrabMove)
    grab.addEventListener('pointerup', onGrabUp)
    return () => {
      cancelAnimationFrame(animation)
      clearTimeout(openTimer)
      clearTimeout(hintTimer)
      clearTimeout(messageTimer)
      removeEventListener('pointermove', onPointerMove)
      document.documentElement.removeEventListener('pointerleave', onPointerLeave)
      page.removeEventListener('pointerdown', onPointerDown)
      removeEventListener('pointerup', onPointerUp)
      removeEventListener('keydown', onKeyDown)
      grab.removeEventListener('pointerdown', onGrabDown)
      grab.removeEventListener('pointermove', onGrabMove)
      grab.removeEventListener('pointerup', onGrabUp)
    }
  }, [elements])

  return { frame, hint, holding, message, on, touch }
}
