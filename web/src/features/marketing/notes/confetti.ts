/**
 * Confetti for finding every note: two cannons fire from the left and right
 * edges toward the middle of the screen. Draws on a throwaway canvas that
 * removes itself when done or when the returned stop function runs.
 */
const FRAMES = 260
const FADE_FRAMES = 45
const PIECES = 260

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

export function fireConfetti(): () => void {
  const canvas = document.createElement('canvas')
  canvas.setAttribute('aria-hidden', 'true')
  canvas.className = 'pointer-events-none fixed inset-0 z-30 h-screen w-screen'
  document.body.append(canvas)
  const context = canvas.getContext('2d')
  if (!context) {
    canvas.remove()
    return () => undefined
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
  const pieces = Array.from({ length: PIECES }, (_, index) => cannonPiece(index, width, height, colors))

  let frame = 0
  let animation = 0
  const draw = () => {
    frame++
    context.clearRect(0, 0, width, height)
    context.globalAlpha = Math.max(0, Math.min(1, (FRAMES - frame) / FADE_FRAMES))
    for (const piece of pieces) {
      if (frame < piece.delay) continue
      piece.vx *= .985
      piece.vy = Math.min(piece.vy * .985 + .32, 4.2)
      piece.tilt += piece.wobble
      piece.x += piece.vx + Math.sin(piece.tilt) * .8
      piece.y += piece.vy
      piece.rotation += piece.spin
      context.save()
      context.translate(piece.x, piece.y)
      context.rotate(piece.rotation)
      context.scale(1, Math.cos(piece.tilt))
      context.fillStyle = piece.color
      context.fillRect(-piece.width / 2, -piece.height / 2, piece.width, piece.height)
      context.restore()
    }
    if (frame < FRAMES) animation = requestAnimationFrame(draw)
    else canvas.remove()
  }
  animation = requestAnimationFrame(draw)
  return () => {
    cancelAnimationFrame(animation)
    canvas.remove()
  }
}

/** A piece fired from low on one side edge, angled up and in, fast enough to
 * carry it to about the middle of the screen before it falls. */
function cannonPiece(index: number, width: number, height: number, colors: string[]): Piece {
  const fromLeft = index % 2 === 0
  const angle = (18 + Math.random() * 40) * Math.PI / 180
  const reach = Math.min(1.3, Math.max(.6, width / 1400))
  const speed = (13 + Math.random() * 14) * reach
  const strip = Math.random() < .25
  return {
    x: fromLeft ? -8 : width + 8,
    y: height * (.62 + Math.random() * .22),
    vx: Math.cos(angle) * speed * (fromLeft ? 1 : -1),
    vy: -Math.sin(angle) * speed,
    width: strip ? 3 : 8 + Math.random() * 6,
    height: strip ? 14 + Math.random() * 8 : 5 + Math.random() * 4,
    rotation: Math.random() * Math.PI * 2,
    spin: (Math.random() - .5) * .3,
    tilt: Math.random() * Math.PI * 2,
    wobble: .08 + Math.random() * .1,
    color: colors[index % colors.length] ?? 'currentColor',
    delay: Math.floor(Math.random() * 18),
  }
}
