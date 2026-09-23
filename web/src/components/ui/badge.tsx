import * as React from "react"

import { cn } from "@/lib/utils"

const base = "group/badge inline-flex h-5 w-fit shrink-0 items-center justify-center gap-1 overflow-hidden rounded-md border px-1.5 py-0.5 whitespace-nowrap focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 aria-invalid:border-destructive aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 [&>svg]:pointer-events-none [&>svg]:size-3!"

const variants = {
  outline: "border-border bg-background text-foreground [a]:hover:bg-muted [a]:hover:text-muted-foreground",
  neutral: "border-transparent bg-muted text-muted-foreground",
  success: "border-success-border bg-success-soft text-success-strong",
  warning: "border-warning-border bg-warning-soft text-warning-strong",
  danger: "border-danger-border bg-danger-soft text-danger-strong",
  info: "border-info-border bg-info-soft text-info-strong",
} as const

export type BadgeVariant = keyof typeof variants

function Badge({
  className,
  stamp = false,
  variant = "outline",
  ...props
}: React.ComponentProps<"span"> & { stamp?: boolean; variant?: BadgeVariant }) {
  return (
    <span
      data-slot="badge"
      data-variant={variant}
      className={cn(base, variants[variant], stamp ? "label-mono rounded-full px-2" : "text-xs font-medium", className)}
      {...props}
    />
  )
}

export { Badge }
