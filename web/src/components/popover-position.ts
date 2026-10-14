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
  if (anchorLeft + panelWidth <= viewportWidth) return 0
  const endAligned = anchorWidth - panelWidth
  if (anchorLeft + endAligned >= 0) return endAligned
  return Math.max(viewportWidth - panelWidth - anchorLeft, -anchorLeft)
}
