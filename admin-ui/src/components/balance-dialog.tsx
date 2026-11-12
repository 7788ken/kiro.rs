import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Progress } from '@/components/ui/progress'
import { useCredentialBalance } from '@/hooks/use-credentials'
import { parseError } from '@/lib/utils'
import { AlertCircle, Loader2, Wallet } from 'lucide-react'

interface BalanceDialogProps { credentialId: number | null; open: boolean; onOpenChange: (open: boolean) => void }
export function BalanceDialog({ credentialId, open, onOpenChange }: BalanceDialogProps) {
  const { data: balance, isLoading, error } = useCredentialBalance(credentialId)
  const formatNumber = (num: number) => num.toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 })
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent className="sm:max-w-md"><DialogHeader><DialogTitle>凭据 #{credentialId} 余额</DialogTitle><DialogDescription>查看订阅等级、用量和下一次额度重置时间。</DialogDescription></DialogHeader>
    {isLoading && <div className="flex min-h-40 items-center justify-center"><Loader2 className="h-6 w-6 animate-spin text-primary" /></div>}
    {error && (() => { const parsed = parseError(error); return <div className="space-y-3 py-6 text-center"><AlertCircle className="mx-auto h-7 w-7 text-destructive" /><p className="text-sm font-medium text-destructive">{parsed.title}</p>{parsed.detail && <p className="text-sm leading-6 text-muted-foreground">{parsed.detail}</p>}</div> })()}
    {balance && <div className="space-y-5"><div className="flex items-center gap-3 rounded-xl bg-primary/[0.06] p-4"><div className="flex h-10 w-10 items-center justify-center rounded-lg bg-card text-primary"><Wallet className="h-5 w-5" /></div><div><p className="text-xs text-muted-foreground">订阅等级</p><p className="mt-1 font-semibold">{balance.subscriptionTitle || '未知订阅类型'}</p></div></div><div className="space-y-2"><div className="flex justify-between text-sm"><span className="text-muted-foreground">已使用 <span className="text-foreground tabular-nums">{formatNumber(balance.currentUsage)}</span></span><span className="text-muted-foreground">限额 <span className="text-foreground tabular-nums">{formatNumber(balance.usageLimit)}</span></span></div><Progress value={balance.usagePercentage} /><p className="text-right text-xs text-muted-foreground tabular-nums">{balance.usagePercentage.toFixed(1)}% 已使用</p></div><div className="grid gap-4 border-t pt-4 text-sm sm:grid-cols-2"><div><p className="text-muted-foreground">剩余额度</p><p className="mt-1.5 font-semibold text-success tabular-nums">{formatNumber(balance.remaining)}</p></div><div><p className="text-muted-foreground">下次重置</p><p className="mt-1.5 font-medium">{balance.nextResetAt ? new Date(balance.nextResetAt * 1000).toLocaleString('zh-CN') : '未知'}</p></div></div></div>}
  </DialogContent></Dialog>
}
