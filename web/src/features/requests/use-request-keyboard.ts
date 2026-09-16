import type { RequestQueueItemResponse, RequestQueueSection } from '@/api/types.generated'
import { isTypingTarget } from '@/lib/keyboard-target'
import { useEffect } from 'react'
import type { RequestAttentionCommand } from './request-attention-api'

const ROW_LINK = '.request-workspace-row-link'
const ROW = '.request-workspace-row'

/**
 * Single-key driving for the inbox. j and k move focus between rows; e, s
 * and c act on the focused row, or the open request when nothing in the
 * list has focus. Keys stay out of inputs, editors, dialogs and menus.
 */
export function useRequestKeyboard({
  onAction,
  onCollapseToggle,
  rows,
  selectedId,
}: {
  onAction: (item: RequestQueueItemResponse, command: RequestAttentionCommand) => void
  onCollapseToggle: () => void
  rows: Map<string, { item: RequestQueueItemResponse; section: RequestQueueSection }>
  selectedId?: string
}) {
  useEffect(() => {
    function handle(event: KeyboardEvent) {
      if (event.defaultPrevented || event.isComposing) return
      if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return
      if (isTypingTarget(event.target)) return
      if (document.querySelector(':popover-open, [role="dialog"], [role="alertdialog"]')) return

      const links = [...document.querySelectorAll<HTMLElement>(ROW_LINK)].filter(
        (link) => link.offsetParent !== null,
      )
      const focusedRow = document.activeElement?.closest<HTMLElement>(ROW) ?? null
      const focusedIndex = links.findIndex((link) => focusedRow?.contains(link))

      if (event.key === 'j' || event.key === 'k') {
        if (!links.length) return
        event.preventDefault()
        const step = event.key === 'j' ? 1 : -1
        const next =
          focusedIndex === -1
            ? links.findIndex((link) => link.getAttribute('aria-current') === 'page') + (step === 1 ? 1 : -1)
            : focusedIndex + step
        const target = links[Math.min(links.length - 1, Math.max(0, next))]
        target.focus({ preventScroll: true })
        target.scrollIntoView({ block: 'nearest' })
        return
      }

      if (event.key === '[') {
        event.preventDefault()
        onCollapseToggle()
        return
      }

      const targetId = focusedRow?.dataset.requestId ?? selectedId
      const row = targetId ? rows.get(targetId) : undefined
      if (!row) return
      const { item, section } = row

      if (event.key === 'e' && section === 'active' && item.attention.can_set_aside) {
        event.preventDefault()
        onAction(item, { action: 'settle' })
      } else if (event.key === 's' && section === 'active' && item.attention.can_set_aside) {
        const trigger = document.querySelector<HTMLElement>(
          `${ROW}[data-request-id="${CSS.escape(item.request.id)}"] [aria-label^="Snooze request"]`,
        )
        if (!trigger) return
        event.preventDefault()
        trigger.click()
      } else if (event.key === 'c' && section === 'unclaimed' && item.attention.can_claim) {
        event.preventDefault()
        onAction(item, { action: 'claim' })
      }
    }
    document.addEventListener('keydown', handle)
    return () => document.removeEventListener('keydown', handle)
  }, [onAction, onCollapseToggle, rows, selectedId])
}
