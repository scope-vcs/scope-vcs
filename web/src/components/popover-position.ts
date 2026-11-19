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
  const lastPanelLeft = viewportWidth - VIEWPORT_GUTTER - panelWidth
  if (anchorLeft <= lastPanelLeft) return Math.max(VIEWPORT_GUTTER, anchorLeft) - anchorLeft
  const endAligned = anchorLeft + anchorWidth - panelWidth
  if (endAligned >= VIEWPORT_GUTTER && endAligned <= lastPanelLeft) return endAligned - anchorLeft
  return Math.max(VIEWPORT_GUTTER, lastPanelLeft) - anchorLeft
}
