import { cn } from '@/lib/utils'
import { scopeBrand } from '@/lib/brand'

export function ScopeLogo({ className }: { className?: string }) {
  return (
    <img
      alt="Scope"
      className={cn('block h-6 w-auto shrink-0 brightness-0 dark:invert', className)}
      decoding="async"
      height={24}
      src={scopeBrand.lockup}
      width={93.3617}
    />
  )
}

export function ScopeMark({ className }: { className?: string }) {
  return (
    <img
      alt=""
      aria-hidden
      className={cn('block size-5 shrink-0 brightness-0 dark:invert', className)}
      decoding="async"
      height={24}
      src={scopeBrand.mark}
      width={24}
    />
  )
}
