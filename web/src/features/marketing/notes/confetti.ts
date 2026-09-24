/**
 * Confetti for finding every note. Normally two cannons fire from the bottom
 * corners; with reduced motion, still pieces fade in along both edges and fade
 * out again. Draws on a throwaway canvas that removes itself.
 */
const CANNON_FRAMES = 260
const CALM_FRAMES = 150
const FADE_FRAMES = 45

interface Piece {
  x: number
  y: number
  vx: number
  vy: number
  width: number
  height: number
  rotation: number
  spin: number
  tilt: number
  wobble: number
  color: string
  delay: number
}

export function fireConfetti(): void {
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches
  const canvas = document.createElement('canvas')
  canvas.setAttribute('aria-hidden', 'true')
  canvas.className = 'pointer-events-none fixed inset-0 z-30 h-screen w-screen'
  document.body.append(canvas)
  const context = canvas.getContext('2d')
  if (!context) {
    canvas.remove()
    return
  }
  const scale = devicePixelRatio || 1
  const width = innerWidth
  const height = innerHeight
  canvas.width = width * scale
  canvas.height = height * scale
  context.scale(scale, scale)

  const tokens = getComputedStyle(document.documentElement)
  const colors = ['--success', '--success-strong', '--success-border', '--foreground', '--warning', '--warning-strong']
    .map((token) => tokens.getPropertyValue(token).trim())
  const pieces = Array.from({ length: reduced ? 90 : 260 }, (_, index) => reduced ? calmPiece(index, width, height, colors) : cannonPiece(index, width, height, colors))
  const frames = reduced ? CALM_FRAMES : CANNON_FRAMES

  let frame = 0
  const draw = () => {
    frame++
    context.clearRect(0, 0, width, height)
    const fadeOut = Math.max(0, Math.min(1, (frames - frame) / FADE_FRAMES))
    for (const piece of pieces) {
      if (frame < piece.delay) continue
      if (!reduced) {
        piece.vx *= .985
        piece.vy = Math.min(piece.vy * .985 + .38, 4.2)
        piece.tilt += piece.wobble
        piece.x += piece.vx + Math.sin(piece.tilt) * .8
        piece.y += piece.vy
        piece.rotation += piece.spin
      }
      const fadeIn = reduced ? Math.min(1, (frame - piece.delay) / 20) : 1
      context.save()
      context.globalAlpha = fadeIn * fadeOut
      context.translate(piece.x, piece.y)
      context.rotate(piece.rotation)
      context.scale(1, Math.cos(piece.tilt))
      context.fillStyle = piece.color
      context.fillRect(-piece.width / 2, -piece.height / 2, piece.width, piece.height)
      context.restore()
    }
    if (frame < frames) requestAnimationFrame(draw)
    else canvas.remove()
  }
  requestAnimationFrame(draw)
}

function cannonPiece(index: number, width: number, height: number, colors: string[]): Piece {
  const fromLeft = index % 2 === 0
  const angle = (58 + Math.random() * 26) * Math.PI / 180
  const speed = (15 + Math.random() * 13) * Math.min(1.25, Math.max(.8, height / 900))
  const strip = Math.random() < .25
  return {
    x: fromLeft ? 0 : width,
    y: height + 6,
    vx: Math.cos(angle) * speed * (fromLeft ? 1 : -1),
    vy: -Math.sin(angle) * speed,
    width: strip ? 3 : 8 + Math.random() * 6,
    height: strip ? 14 + Math.random() * 8 : 5 + Math.random() * 4,
    rotation: Math.random() * Math.PI * 2,
    spin: (Math.random() - .5) * .3,
    tilt: Math.random() * Math.PI * 2,
    wobble: .08 + Math.random() * .1,
    color: colors[index % colors.length] ?? 'currentColor',
    delay: Math.floor(Math.random() * 22),
  }
}

function calmPiece(index: number, width: number, height: number, colors: string[]): Piece {
  const band = width * .14
  const fromLeft = index % 2 === 0
  return {
    x: fromLeft ? Math.random() * band : width - Math.random() * band,
    y: Math.random() * height,
    vx: 0,
    vy: 0,
    width: 8 + Math.random() * 6,
    height: 5 + Math.random() * 4,
    rotation: Math.random() * Math.PI * 2,
    spin: 0,
    tilt: Math.random() * Math.PI * 2,
    wobble: 0,
    color: colors[index % colors.length] ?? 'currentColor',
    delay: Math.floor(Math.random() * 30),
  }
}
