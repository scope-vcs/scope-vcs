import * as React from "react"

import { AlertDescription } from "@/components/ui/alert-description"
import { AlertTitle } from "@/components/ui/alert-title"
import { cn } from "@/lib/utils"

const base = "group/alert relative grid w-full gap-0.5 rounded-lg border px-3 py-2.5 text-left text-sm has-[>svg]:grid-cols-[auto_1fr] has-[>svg]:gap-x-2 *:[svg]:row-span-2 *:[svg]:translate-y-0.5 *:[svg]:text-current *:[svg:not([class*='size-'])]:size-4"

const variants = {
  default: "bg-background text-foreground",
  destructive: "border-destructive/30 bg-destructive/10 text-destructive *:data-[slot=alert-description]:text-destructive/90 *:[svg]:text-current",
} as const

function Alert({
  className,
  variant = "default",
  ...props
}: React.ComponentProps<"div"> & { variant?: keyof typeof variants }) {
  return (
    <div
      data-slot="alert"
      role={variant === "destructive" ? "alert" : undefined}
      className={cn(base, variants[variant], className)}
      {...props}
    />
  )
}

export { Alert, AlertTitle, AlertDescription }
