import type { LucideIcon } from 'lucide-react'
import { Card, CardContent } from '@/components/ui/card'
import { cn } from '@/lib/utils'

export function PageSection({ title, description, actions, children }: { title: string; description?: string; actions?: React.ReactNode; children: React.ReactNode }) {
  return <section className="space-y-4"><div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between"><div><h2 className="text-base font-semibold tracking-[-0.01em]">{title}</h2>{description && <p className="mt-1 text-sm text-muted-foreground">{description}</p>}</div>{actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}</div>{children}</section>
}

export function MetricCard({ label, value, detail, icon: Icon, tone = 'default' }: { label: string; value: React.ReactNode; detail?: React.ReactNode; icon: LucideIcon; tone?: 'default' | 'success' | 'accent' }) {
  return <Card className="shadow-none"><CardContent className="p-5"><div className="flex items-start justify-between gap-4"><div className="min-w-0"><p className="text-[13px] font-medium text-muted-foreground">{label}</p><div className={cn('mt-3 text-[28px] font-semibold leading-none tracking-[-0.03em] tabular-nums', tone === 'success' && 'text-success', tone === 'accent' && 'text-primary')}>{value}</div></div><div className={cn('flex h-9 w-9 shrink-0 items-center justify-center rounded-lg', tone === 'success' ? 'bg-success/10 text-success' : 'bg-primary/[0.07] text-primary')}><Icon className="h-[17px] w-[17px]" /></div></div>{detail && <div className="mt-3 text-xs leading-5 text-muted-foreground">{detail}</div>}</CardContent></Card>
}

export function EmptyState({ icon: Icon, title, description, action }: { icon: LucideIcon; title: string; description: string; action?: React.ReactNode }) {
  return <div className="flex flex-col items-center px-6 py-16 text-center"><div className="flex h-11 w-11 items-center justify-center rounded-xl bg-muted text-muted-foreground"><Icon className="h-5 w-5" /></div><h3 className="mt-4 text-sm font-medium">{title}</h3><p className="mt-2 max-w-sm text-pretty text-sm leading-6 text-muted-foreground">{description}</p>{action && <div className="mt-5">{action}</div>}</div>
}

export const tableClass = 'w-full text-sm'
export const tableHeadClass = 'border-b bg-muted/30 text-left text-xs font-medium text-muted-foreground'
export const tableRowClass = 'border-b transition-colors last:border-0 hover:bg-muted/20'
export const tableCellClass = 'px-5 py-4 align-middle'
