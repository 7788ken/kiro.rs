import * as React from 'react'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '@/lib/utils'

const badgeVariants = cva('inline-flex shrink-0 items-center gap-1 rounded-md border px-2 py-0.5 text-[11px] font-medium leading-4', {
  variants: {
    variant: {
      default: 'border-primary/15 bg-primary/[0.07] text-primary',
      secondary: 'border-transparent bg-secondary text-secondary-foreground',
      destructive: 'border-destructive/15 bg-destructive/[0.07] text-destructive',
      outline: 'border-border bg-card text-muted-foreground',
      success: 'border-success/15 bg-success/[0.07] text-success',
      warning: 'border-warning/20 bg-warning/10 text-warning',
    },
  },
  defaultVariants: { variant: 'default' },
})

export interface BadgeProps extends React.HTMLAttributes<HTMLDivElement>, VariantProps<typeof badgeVariants> {}
function Badge({ className, variant, ...props }: BadgeProps) {
  return <div className={cn(badgeVariants({ variant }), className)} {...props} />
}

export { Badge, badgeVariants }
