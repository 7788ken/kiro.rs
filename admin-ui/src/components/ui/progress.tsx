import * as React from 'react'
import { cn } from '@/lib/utils'

interface ProgressProps extends React.HTMLAttributes<HTMLDivElement> { value?: number; max?: number }
const Progress = React.forwardRef<HTMLDivElement, ProgressProps>(({ className, value = 0, max = 100, ...props }, ref) => {
  const percentage = max > 0 ? Math.min(Math.max((value / max) * 100, 0), 100) : 0
  return <div ref={ref} role="progressbar" aria-valuenow={percentage} aria-valuemin={0} aria-valuemax={100} className={cn('relative h-2 w-full overflow-hidden rounded-full bg-secondary', className)} {...props}><div className={cn('h-full rounded-full transition-[width,background-color] duration-300', percentage > 80 ? 'bg-destructive' : percentage > 60 ? 'bg-warning' : 'bg-primary')} style={{ width: `${percentage}%` }} /></div>
})
Progress.displayName = 'Progress'
export { Progress }
