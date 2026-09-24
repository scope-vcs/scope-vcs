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
import { LoaderCircle } from 'lucide-react'
import type { FormEvent, ReactNode, RefObject } from 'react'

export function RequestConfirmDialog({
  children,
  confirmLabel,
  destructive = false,
  onConfirm,
  onOpenChange,
  open,
  pending,
  returnFocus,
  title,
}: {
  children: ReactNode
  confirmLabel: string
  destructive?: boolean
  onConfirm: () => Promise<boolean>
  onOpenChange: (open: boolean) => void
  open: boolean
  pending: boolean
  /** Where focus goes on close when no Radix trigger opened the dialog. */
  returnFocus?: RefObject<HTMLElement | null>
  title: string
}) {
  async function confirm(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!pending && await onConfirm()) onOpenChange(false)
  }

  return (
    <AlertDialog
      onOpenChange={(nextOpen) => {
        if (!pending) onOpenChange(nextOpen)
      }}
      open={open}
    >
      <AlertDialogContent
        asChild
        onCloseAutoFocus={(event) => {
          if (!returnFocus?.current) return
          event.preventDefault()
          returnFocus.current.focus()
        }}
      >
        <form onSubmit={(event) => void confirm(event)}>
          <AlertDialogHeader>
            <AlertDialogTitle>{title}</AlertDialogTitle>
            <AlertDialogDescription asChild>
              <div className="grid gap-2">{children}</div>
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending} size="sm">
              Cancel
            </AlertDialogCancel>
            <Button
              disabled={pending}
              size="sm"
              type="submit"
              variant={destructive ? 'destructive' : 'default'}
            >
              {pending ? <LoaderCircle className="animate-spin" /> : null}
              {confirmLabel}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}
