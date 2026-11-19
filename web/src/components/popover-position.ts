const VIEWPORT_GUTTER = 16

export function autoPopoverOffset({
  anchorLeft,
  anchorWidth,
  panelWidth,
  viewportWidth,
}: {
  anchorLeft: number
  anchorWidth: number
  panelWidth: number
  viewportWidth: number
}) {
  if (anchorLeft + panelWidth <= viewportWidth - VIEWPORT_GUTTER) return 0
  const endAligned = anchorWidth - panelWidth
  if (anchorLeft + endAligned >= VIEWPORT_GUTTER) return endAligned
  return Math.max(viewportWidth - VIEWPORT_GUTTER - panelWidth - anchorLeft, VIEWPORT_GUTTER - anchorLeft)
}
