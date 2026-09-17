/**
 * Whether a key press landed somewhere the user is typing, so single-key
 * shortcuts must stay out of the way.
 */
export function isTypingTarget(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    target.closest(
      'input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"]',
    ) !== null
  )
}
