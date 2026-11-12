import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { Plus } from 'lucide-react'
import { createUser } from '@/api/accounts'
import { extractErrorMessage } from '@/lib/utils'
import type { Role } from '@/types/accounts'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { KeyDialog } from '@/components/key-dialog'

export function CreateUserDialog({ open, onOpenChange, isSuperAdmin }: { open: boolean; onOpenChange: (open: boolean) => void; isSuperAdmin: boolean }) {
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [role, setRole] = useState<Role>('user')
  const [initialBalance, setInitialBalance] = useState('0')
  const [apiKey, setApiKey] = useState('')
  const queryClient = useQueryClient()
  const mutation = useMutation({
    mutationFn: createUser,
    onSuccess: result => {
      setApiKey(result.apiKey)
      onOpenChange(false)
      setUsername('')
      setPassword('')
      setRole('user')
      setInitialBalance('0')
      queryClient.invalidateQueries({ queryKey: ['users'] })
      queryClient.invalidateQueries({ queryKey: ['transactions'] })
    },
    onError: error => toast.error(extractErrorMessage(error)),
  })

  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    const balance = Number(initialBalance)
    if (!Number.isFinite(balance) || balance < 0) return toast.error('初始积分必须是非负数字')
    if (password.length < 10) return toast.error('密码至少需要 10 个字符')
    mutation.mutate({ username: username.trim(), password, role, initialBalance: balance })
  }

  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent>
          <DialogHeader><DialogTitle>创建用户</DialogTitle><DialogDescription>创建后立即生成独立 API Key，初始密码请通过安全渠道交给用户。</DialogDescription></DialogHeader>
          <form onSubmit={submit} className="space-y-4">
            <div className="space-y-2"><label className="text-sm font-medium" htmlFor="new-username">用户名</label><Input id="new-username" value={username} onChange={event => setUsername(event.target.value)} placeholder="3-40 位字母、数字或 _ - . @" required /></div>
            <div className="space-y-2"><label className="text-sm font-medium" htmlFor="new-password">初始密码</label><Input id="new-password" type="password" value={password} onChange={event => setPassword(event.target.value)} placeholder="至少 10 个字符" minLength={10} required /></div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2"><label className="text-sm font-medium" htmlFor="new-role">角色</label><select id="new-role" className="flex h-10 w-full rounded-md border border-input bg-background px-3 text-sm" value={role} onChange={event => setRole(event.target.value as Role)}><option value="user">用户</option>{isSuperAdmin && <option value="admin">管理员</option>}</select></div>
              <div className="space-y-2"><label className="text-sm font-medium" htmlFor="initial-credits">初始积分</label><Input id="initial-credits" type="number" min="0" step="0.000001" value={initialBalance} onChange={event => setInitialBalance(event.target.value)} /></div>
            </div>
            <DialogFooter><Button type="button" variant="outline" onClick={() => onOpenChange(false)}>取消</Button><Button disabled={mutation.isPending}><Plus />{mutation.isPending ? '创建中...' : '创建用户'}</Button></DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
      <KeyDialog apiKey={apiKey} onClose={() => setApiKey('')} title="用户创建成功" />
    </>
  )
}
