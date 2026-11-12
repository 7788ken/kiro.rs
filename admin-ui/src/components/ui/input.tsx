import * as React from 'react'
import { cn } from '@/lib/utils'

export interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {}

const Input = React.forwardRef<HTMLInputElement, InputProps>(({ className, type, ...props }, ref) => (
  <input type={type} className={cn('flex h-10 w-full rounded-lg border border-input bg-card px-3 py-2 text-sm shadow-none ring-offset-background transition-[border-color,box-shadow] duration-150 file:border-0 file:bg-transparent file:text-sm file:font-medium file:text-foreground placeholder:text-muted-foreground/80 focus-visible:border-primary/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/15 disabled:cursor-not-allowed disabled:opacity-50', className)} ref={ref} {...props} />
))
Input.displayName = 'Input'

export { Input }

export const formControlClass = 'flex h-10 w-full rounded-lg border border-input bg-card px-3 text-sm text-foreground shadow-none outline-none transition-[border-color,box-shadow] duration-150 focus:border-primary/50 focus:ring-2 focus:ring-primary/15 disabled:cursor-not-allowed disabled:opacity-50'
export const textareaClass = 'flex min-h-[180px] w-full resize-y rounded-lg border border-input bg-card px-3 py-2.5 font-mono text-sm leading-6 outline-none transition-[border-color,box-shadow] duration-150 placeholder:font-sans placeholder:text-muted-foreground/80 focus:border-primary/50 focus:ring-2 focus:ring-primary/15 disabled:cursor-not-allowed disabled:opacity-50'
