import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { cn } from '@/lib/utils'
import { AlertCircle } from 'lucide-react'
import type { ReactNode } from 'react'

export function PageErrorAlert({
  children,
  className,
  title,
}: {
  children: ReactNode
  className?: string
  title: string
}) {
  return (
    <Alert className={cn('mt-6', className)} variant="destructive">
      <AlertCircle className="size-4" />
      <AlertTitle>{title}</AlertTitle>
      <AlertDescription>{children}</AlertDescription>
    </Alert>
  )
}
