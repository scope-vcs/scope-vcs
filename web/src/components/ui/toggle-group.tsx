"use client"

import * as React from "react"
import * as ToggleGroupPrimitive from "@radix-ui/react-toggle-group"

import { buttonVariants } from "@/components/ui/button-variants"
import { cn } from "@/lib/utils"

const TOGGLE_GROUP_CLASS =
  "group/toggle-group inline-flex w-fit items-center rounded-lg border border-border bg-muted p-0.5"
const TOGGLE_GROUP_ITEM_CLASS =
  "min-w-0 border-transparent text-muted-foreground shadow-none data-[state=on]:bg-card data-[state=on]:text-foreground data-[state=on]:shadow-[var(--shadow-card)]"

function ToggleGroup({
  className,
  children,
  ...props
}: React.ComponentProps<typeof ToggleGroupPrimitive.Root>) {
  return (
    <ToggleGroupPrimitive.Root
      data-slot="toggle-group"
      className={cn(
        TOGGLE_GROUP_CLASS,
        className
      )}
      {...props}
    >
      {children}
    </ToggleGroupPrimitive.Root>
  )
}

function ToggleGroupItem({
  className,
  children,
  ...props
}: React.ComponentProps<typeof ToggleGroupPrimitive.Item>) {
  return (
    <ToggleGroupPrimitive.Item
      data-slot="toggle-group-item"
      className={cn(
        buttonVariants({ variant: "ghost", size: "sm" }),
        TOGGLE_GROUP_ITEM_CLASS,
        className
      )}
      {...props}
    >
      {children}
    </ToggleGroupPrimitive.Item>
  )
}

export { ToggleGroup, ToggleGroupItem }
