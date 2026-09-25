import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { AlertTriangle, LoaderCircle, Trash2 } from 'lucide-react'
import type { FormEvent, ReactNode } from 'react'
import { useId, useState } from 'react'

/**
 * A permanent deletion in two steps: a warning, then typing `confirmation`
 * to enable the destructive button.
 */
export function TypedConfirmationDialog({
  confirmLabel,
  confirmation,
  error,
  onCancel,
  onConfirm,
  purpose,
  subject,
  title,
  warning,
}: {
  confirmLabel: string
  /** The text the user types to confirm. */
  confirmation: string
  error: ReactNode
  onCancel: () => void
  onConfirm: () => Promise<void>
  /** Completes "Type <confirmation> to …". */
  purpose: string
  subject: string
  title: string
  warning: ReactNode
}) {
  const inputId = useId()
  const [confirmed, setConfirmed] = useState(false)
  const [typed, setTyped] = useState('')
  const [busy, setBusy] = useState(false)
  const canConfirm = typed === confirmation

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!confirmed) {
      setConfirmed(true)
      return
    }

    if (!canConfirm || busy) {
      return
    }

    setBusy(true)
    try {
      await onConfirm()
    } catch {
      // The caller reports the failure through `error`.
    } finally {
      setBusy(false)
    }
  }

  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) {
          onCancel()
        }
      }}
    >
      <AlertDialogContent asChild>
        <form onSubmit={(event) => void submit(event)}>
          <AlertDialogHeader className="grid-cols-[auto_minmax(0,1fr)] gap-x-3">
            <div className="row-span-2 flex size-9 shrink-0 items-center justify-center rounded-lg bg-destructive/10 text-destructive">
              <AlertTriangle className="size-4" />
            </div>
            <AlertDialogTitle>{title}</AlertDialogTitle>
            <div className="break-all font-mono text-xs leading-5 text-muted-foreground">
              {subject}
            </div>
          </AlertDialogHeader>

          {!confirmed ? (
            <AlertDialogDescription>{warning}</AlertDialogDescription>
          ) : (
            <div className="space-y-2">
              <AlertDialogDescription>
                Type{' '}
                <span className="font-mono text-foreground">{confirmation}</span>{' '}
                to {purpose}.
              </AlertDialogDescription>
              <Input
                aria-label={`Type ${confirmation} to ${purpose}`}
                autoFocus
                className="font-mono"
                id={inputId}
                onChange={(event) => setTyped(event.target.value)}
                value={typed}
              />
            </div>
          )}

          {error && <div className="text-sm text-destructive" role="alert">{error}</div>}

          <AlertDialogFooter>
            {!confirmed ? (
              <>
                <AlertDialogCancel size="sm" variant="secondary">
                  Cancel
                </AlertDialogCancel>
                <Button size="sm" type="submit">
                  Continue
                </Button>
              </>
            ) : (
              <>
                <Button
                  disabled={busy}
                  onClick={() => setConfirmed(false)}
                  size="sm"
                  type="button"
                  variant="secondary"
                >
                  Back
                </Button>
                <Button
                  disabled={!canConfirm || busy}
                  size="sm"
                  type="submit"
                  variant="destructive"
                >
                  {busy ? (
                    <LoaderCircle className="size-3.5 animate-spin" />
                  ) : (
                    <Trash2 className="size-3.5" />
                  )}
                  <span>{confirmLabel}</span>
                </Button>
              </>
            )}
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}
