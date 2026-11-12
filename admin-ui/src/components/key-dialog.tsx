import { useState } from 'react'
import { Check, Copy, KeyRound } from 'lucide-react'
import { toast } from 'sonner'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'

export function KeyDialog({ apiKey, onClose, title = '新的 API Key' }: { apiKey: string; onClose: () => void; title?: string }) {
  const [copied, setCopied] = useState(false)
  const copy = async () => {
    try {
      if (navigator.clipboard && window.isSecureContext) {
        await navigator.clipboard.writeText(apiKey)
      } else {
        const textarea = document.createElement('textarea')
        textarea.value = apiKey
        textarea.style.position = 'fixed'
        textarea.style.opacity = '0'
        document.body.appendChild(textarea)
        textarea.select()
        if (!document.execCommand('copy')) throw new Error('copy failed')
        textarea.remove()
      }
      setCopied(true)
      toast.success('API Key 已复制')
    } catch { toast.error('复制失败，请选中 Key 后手动复制') }
  }
  return <Dialog open={Boolean(apiKey)} onOpenChange={open => { if (!open) { onClose(); setCopied(false) } }}><DialogContent><DialogHeader><DialogTitle className="flex items-center gap-2"><KeyRound className="h-5 w-5 text-primary" />{title}</DialogTitle><DialogDescription>请立即复制并妥善保存。出于安全考虑，关闭后无法再次查看完整 Key。</DialogDescription></DialogHeader><div className="rounded-lg border border-warning/20 bg-warning/[0.06] p-3"><Input readOnly value={apiKey} aria-label="新的 API Key" className="border-0 bg-transparent px-1 font-mono text-xs shadow-none focus-visible:ring-0" onFocus={event => event.target.select()} /></div><DialogFooter><Button variant="outline" onClick={onClose}>我已保存</Button><Button onClick={copy}>{copied ? <Check /> : <Copy />}{copied ? '已复制' : '复制 Key'}</Button></DialogFooter></DialogContent></Dialog>
}
