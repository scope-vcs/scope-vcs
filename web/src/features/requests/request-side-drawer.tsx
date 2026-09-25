import { Button } from '@/components/ui/button'
import * as Dialog from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import { useId, type ReactNode, type RefObject } from 'react'

/** A right-hand panel over the request page, opened without a Radix trigger. */
export function RequestSideDrawer({
  children,
  description,
  footer,
  icon,
  onOpenChange,
  open,
  returnFocus,
  title,
}: {
  children: ReactNode
  description: string
  footer?: ReactNode
  icon: ReactNode
  onOpenChange: (open: boolean) => void
  open: boolean
  /** Where focus goes on close. */
  returnFocus: RefObject<HTMLElement | null>
  title: string
}) {
  const descriptionId = useId()
  return (
    <Dialog.Root onOpenChange={onOpenChange} open={open}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-background/80 backdrop-blur-sm" />
        <Dialog.Content
          aria-describedby={descriptionId}
          className="fixed inset-y-0 right-0 z-50 flex w-[520px] max-w-[90vw] flex-col border-l border-[var(--border-strong)] bg-background shadow-[var(--shadow-pop)] outline-none"
          onCloseAutoFocus={(event) => {
            if (!returnFocus.current) return
            event.preventDefault()
            returnFocus.current.focus()
          }}
        >
          <div className="flex min-h-16 items-start gap-3 border-b border-border px-5 py-4">
            <span className="mt-0.5 shrink-0 text-muted-foreground [&_svg]:size-4">{icon}</span>
            <div className="min-w-0 flex-1">
              <Dialog.Title className="text-sm font-semibold">{title}</Dialog.Title>
              <Dialog.Description
                className="mt-1 text-xs leading-5 text-muted-foreground"
                id={descriptionId}
              >
                {description}
              </Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <Button
                aria-label={`Close ${title.toLowerCase()}`}
                size="icon-xs"
                type="button"
                variant="ghost"
              >
                <X className="size-3.5" />
              </Button>
            </Dialog.Close>
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto">{children}</div>
          {footer}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}
