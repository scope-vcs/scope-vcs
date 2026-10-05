export function isTypingTarget(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    target.closest(
      'input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"]',
    ) !== null
  )
}
