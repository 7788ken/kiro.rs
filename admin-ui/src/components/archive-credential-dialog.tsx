import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'

interface ArchiveCredentialDialogProps {
  credentialId: number
  open: boolean
  onOpenChange: (open: boolean) => void
  onConfirm: (reason: string) => void
  pending: boolean
}

export function ArchiveCredentialDialog({ credentialId, open, onOpenChange, onConfirm, pending }: ArchiveCredentialDialogProps) {
  const [reason, setReason] = useState('')
  useEffect(() => {
    if (open) setReason('')
  }, [open])
  const inputId = `archive-reason-${credentialId}`
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>归档凭据</DialogTitle>
          <DialogDescription>归档后该凭据停止参与调度并从常规列表隐藏，统计数据保留，可随时恢复。</DialogDescription>
        </DialogHeader>
        <div className="space-y-2">
          <label htmlFor={inputId} className="text-sm font-medium">归档原因（可选）</label>
          <Input id={inputId} value={reason} onChange={event => setReason(event.target.value)} placeholder={`凭据 #${credentialId} 的归档原因`} maxLength={200} />
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={pending}>取消</Button>
          <Button onClick={() => onConfirm(reason.trim())} disabled={pending}>确认归档</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
