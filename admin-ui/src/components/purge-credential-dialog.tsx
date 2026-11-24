import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'

interface PurgeCredentialDialogProps {
  credentialId: number
  open: boolean
  onOpenChange: (open: boolean) => void
  onConfirm: () => void
  pending: boolean
}

export function PurgeCredentialDialog({ credentialId, open, onOpenChange, onConfirm, pending }: PurgeCredentialDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>彻底删除凭据</DialogTitle>
          <DialogDescription>彻底删除凭据 #{credentialId}？该凭据的 Token、统计和累计消耗将一并删除，无法恢复。</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={pending}>取消</Button>
          <Button variant="destructive" onClick={onConfirm} disabled={pending}>彻底删除</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
