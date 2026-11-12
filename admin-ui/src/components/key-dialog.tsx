import { Copy, KeyRound } from 'lucide-react'
import { toast } from 'sonner'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'

export function KeyDialog({ apiKey, onClose, title = '新的 API Key' }: { apiKey: string; onClose: () => void; title?: string }) {
  return (
    <Dialog open={Boolean(apiKey)} onOpenChange={open => { if (!open) onClose() }}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2"><KeyRound className="h-5 w-5" />{title}</DialogTitle>
          <DialogDescription>请立即复制并妥善保存。出于安全考虑，关闭后无法再次查看完整 Key。</DialogDescription>
        </DialogHeader>
        <Input readOnly value={apiKey} className="font-mono text-xs" onFocus={event => event.target.select()} />
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>我已保存</Button>
          <Button onClick={async () => { await navigator.clipboard.writeText(apiKey); toast.success('API Key 已复制') }}><Copy />复制 Key</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
