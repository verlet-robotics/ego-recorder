import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"

import { cn } from "@/lib/utils"

const badgeVariants = cva(
  "inline-flex items-center font-sans font-semibold transition-colors",
  {
    variants: {
      variant: {
        pill: "rounded-[20px] text-[10px] uppercase tracking-[0.12em] px-3.5 py-[5px] bg-surface text-muted-foreground",
        inline:
          "rounded-[6px] text-[9px] uppercase tracking-[0.1em] px-2 py-[2px] bg-surface text-muted-foreground",
        chip: "rounded-[8px] text-[11px] px-2.5 py-1 bg-surface text-muted-foreground",
      },
    },
    defaultVariants: {
      variant: "pill",
    },
  }
)

function Badge({
  className,
  variant,
  ...props
}: React.ComponentProps<"span"> & VariantProps<typeof badgeVariants>) {
  return (
    <span
      data-slot="badge"
      className={cn(badgeVariants({ variant, className }))}
      {...props}
    />
  )
}

export { Badge, badgeVariants }
