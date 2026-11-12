import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogDescription } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Progress } from '@/components/ui/progress'
import { CheckCircle2, Clock3, Loader2, XCircle } from 'lucide-react'

export interface VerifyResult { id: number; status: 'pending' | 'verifying' | 'success' | 'failed'; usage?: string; error?: string }
interface BatchVerifyDialogProps { open: boolean; onOpenChange: (open: boolean) => void; verifying: boolean; progress: { current: number; total: number }; results: Map<number, VerifyResult>; onCancel: () => void }

export function BatchVerifyDialog({ open, onOpenChange, verifying, progress, results, onCancel }: BatchVerifyDialogProps) {
  const resultsArray = Array.from(results.values())
  const successCount = resultsArray.filter(r => r.status === 'success').length
  const failedCount = resultsArray.filter(r => r.status === 'failed').length
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent><DialogHeader><DialogTitle>批量验活</DialogTitle><DialogDescription>逐个检查凭据的订阅信息和剩余额度。</DialogDescription></DialogHeader><div className="space-y-4 py-2">
    <div className="space-y-2"><div className="flex justify-between text-sm"><span>{verifying ? '验活进度' : '验活结果'}</span><span className="tabular-nums">{progress.current} / {progress.total}</span></div><Progress value={progress.current} max={progress.total} /></div>
    {results.size > 0 && <><div className="flex gap-2"><Badge variant="success">成功 {successCount}</Badge><Badge variant={failedCount > 0 ? 'destructive' : 'secondary'}>失败 {failedCount}</Badge></div><div className="max-h-[400px] space-y-2 overflow-y-auto rounded-lg border p-2">{resultsArray.map(result => <div key={result.id} className="rounded-lg bg-muted/45 p-3 text-sm"><div className="flex items-center justify-between gap-3"><div className="flex items-center gap-2"><span className="font-medium">凭据 #{result.id}</span>{result.status === 'success' && result.usage && <Badge variant="secondary">{result.usage}</Badge>}</div>{result.status === 'success' ? <CheckCircle2 className="h-4 w-4 text-success" /> : result.status === 'failed' ? <XCircle className="h-4 w-4 text-destructive" /> : result.status === 'verifying' ? <Loader2 className="h-4 w-4 animate-spin text-primary" /> : <Clock3 className="h-4 w-4 text-muted-foreground" />}</div>{result.error && <p className="mt-2 text-pretty text-xs leading-5 text-destructive">{result.error}</p>}</div>)}</div></>}
    {verifying && <p className="text-pretty text-xs leading-5 text-muted-foreground">每次请求间隔 2 秒。你可以关闭窗口，验活会在后台继续。</p>}
  </div><DialogFooter>{verifying ? <><Button variant="outline" onClick={() => onOpenChange(false)}>后台运行</Button><Button variant="destructive" onClick={onCancel}>取消验活</Button></> : <Button onClick={() => onOpenChange(false)}>关闭</Button>}</DialogFooter></DialogContent></Dialog>
}
