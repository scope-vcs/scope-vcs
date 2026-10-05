export interface LensFrame {
  x: number
  y: number
  r: number
  rotation: number
}

export interface Box {
  left: number
  top: number
  right: number
  bottom: number
}

export const PACE = 1.25

const paced = (ease: number) => 1 - (1 - ease) ** PACE

const FOLLOW_EASE = paced(.2)
const RADIUS_EASE = paced(.13)
const QUICK_RADIUS_EASE = paced(.35)
const IDLE_SPIN = .03 * PACE
const MAX_ZOOM = .08

export function restRadius(viewportWidth: number): number {
  return viewportWidth < 700 ? 112 : 156
}

export function floodRadius(viewportWidth: number, viewportHeight: number): number {
  return Math.hypot(viewportWidth, viewportHeight) * 1.1
}

export function stepLens(
  frame: LensFrame,
  target: { x: number; y: number; r: number },
  { instant, resting }: { instant: boolean; resting: boolean },
): LensFrame {
  const follow = instant ? 1 : FOLLOW_EASE
  const dx = (target.x - frame.x) * follow
  return {
    x: frame.x + dx,
    y: frame.y + (target.y - frame.y) * follow,
    r: frame.r + (target.r - frame.r) * (instant ? QUICK_RADIUS_EASE : RADIUS_EASE),
    rotation: instant ? frame.rotation : frame.rotation + dx * .35 + (resting ? IDLE_SPIN : 0),
  }
}

export function restingPoint(
  anchor: Box,
  page: { left: number; top: number },
  viewport: { width: number; height: number },
  time: number,
  drift: boolean,
): { x: number; y: number } {
  const onScreen = anchor.top > 0 && anchor.bottom < viewport.height
  const x = onScreen ? anchor.left - page.left + 96 : viewport.width * .62 - page.left
  const y = onScreen ? anchor.top - page.top + 44 : viewport.height * .55 - page.top
  if (!drift) return { x, y }
  return { x: x + Math.sin(time * PACE / 1900) * 26, y: y + Math.cos(time * PACE / 2600) * 18 }
}

export function lensZoom(r: number, rest: number, instant: boolean): number {
  if (instant) return 1
  return 1 + MAX_ZOOM * Math.max(0, Math.min(1, 1 - (r - rest) / rest))
}

export function ringOpacity(r: number, rest: number): number {
  return Math.min(1, r / 70) * Math.max(0, 1 - Math.max(0, r - rest * 1.6) / 200)
}

export function tickPaths(r: number): { minor: string; major: string } {
  let minor = ''
  let major = ''
  for (let index = 0; index < 72; index++) {
    const angle = (index / 72) * Math.PI * 2
    const isMajor = index % 6 === 0
    const outer = r + (isMajor ? 14 : 10)
    const inner = r + 6
    const segment = `M${(Math.cos(angle) * inner).toFixed(1)} ${(Math.sin(angle) * inner).toFixed(1)}L${(Math.cos(angle) * outer).toFixed(1)} ${(Math.sin(angle) * outer).toFixed(1)}`
    if (isMajor) major += segment
    else minor += segment
  }
  return { minor, major }
}

export function lensCovers(box: Box, x: number, y: number, r: number, viewport: { width: number; height: number }): boolean {
  if (box.bottom < 0 || box.top > viewport.height || box.right < 0 || box.left > viewport.width) return false
  const nearestX = Math.max(box.left, Math.min(x, box.right))
  const nearestY = Math.max(box.top, Math.min(y, box.bottom))
  return Math.hypot(nearestX - x, nearestY - y) <= r * .45
}
