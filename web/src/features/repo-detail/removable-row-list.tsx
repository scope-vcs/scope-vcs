import { DestructiveActionDialog } from '@/components/destructive-action-dialog'
import { Button } from '@/components/ui/button'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { usePendingActions } from '@/lib/use-pending-actions'
import { LoaderCircle, Trash2 } from 'lucide-react'
import { useState, type ReactNode } from 'react'

export type RowActions = {
  pending: boolean
  remove: () => void
  run: (action: () => Promise<unknown>) => void
}

/**
 * A list whose rows can run one async action at a time and be removed after a
 * confirmation. Owns the shared pending, error, and confirm-target state so
 * member and invite rows cannot drift in how they report a failed call.
 */
export function RemovableRowList<Item>({
  confirm,
  fallbackError,
  itemId,
  items,
  onRemove,
  row,
  rowClassName,
}: {
  confirm: {
    confirmLabel: string
    description: string
    subject: (item: Item) => string
    title: string
  }
  fallbackError: string
  itemId: (item: Item) => string
  items: readonly Item[]
  onRemove: (item: Item) => Promise<unknown>
  row: (item: Item, actions: RowActions) => ReactNode
  rowClassName: string
}) {
  const [error, setError] = useState<string | null>(null)
  const [confirmTarget, setConfirmTarget] = useState<Item | null>(null)
  const { pending, run: runPending } = usePendingActions()

  async function run(item: Item, action: () => Promise<unknown>) {
    const id = itemId(item)
    await runPending(id, async () => {
      setError(null)
      try {
        await action()
      } catch (error) {
        setError(resourceErrorMessage(error, fallbackError))
      }
    })
  }

  async function remove(item: Item) {
    try {
      await run(item, () => onRemove(item))
    } finally {
      setConfirmTarget(null)
    }
  }

  return (
    <div className="space-y-3">
      <ul className="divide-y divide-border">
        {items.map((item) => {
          const id = itemId(item)
          return (
            <li className={rowClassName} key={id}>
              {row(item, {
                pending: pending.has(id),
                remove: () => setConfirmTarget(item),
                run: (action) => void run(item, action),
              })}
            </li>
          )
        })}
      </ul>
      <DestructiveActionDialog
        confirmLabel={confirm.confirmLabel}
        description={confirm.description}
        onConfirm={() => {
          if (confirmTarget !== null) void remove(confirmTarget)
        }}
        onOpenChange={(open) => {
          if (!open && !(confirmTarget !== null && pending.has(itemId(confirmTarget)))) setConfirmTarget(null)
        }}
        open={confirmTarget !== null}
        pending={confirmTarget !== null && pending.has(itemId(confirmTarget))}
        subject={confirmTarget !== null ? confirm.subject(confirmTarget) : ''}
        title={confirm.title}
      />
      {error && <p className="text-sm text-destructive" role="alert">{error}</p>}
    </div>
  )
}

export function RemoveButton({
  label,
  onClick,
  pending,
}: {
  label: string
  onClick: () => void
  pending: boolean
}) {
  return (
    <Button
      disabled={pending}
      onClick={onClick}
      size="sm"
      type="button"
      variant="secondary"
    >
      {pending ? (
        <LoaderCircle className="size-3.5 animate-spin" />
      ) : (
        <Trash2 className="size-3.5" />
      )}
      <span>{label}</span>
    </Button>
  )
}
