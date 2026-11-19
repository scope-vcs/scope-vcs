export function isTypingTarget(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    target.closest(
      'input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"]',
    ) !== null
  )
}

export function overlayIsOpen() {
  return document.querySelector(':popover-open, dialog[open], [role="dialog"], [role="alertdialog"]') !== null
}
