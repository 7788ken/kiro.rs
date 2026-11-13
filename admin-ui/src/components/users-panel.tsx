import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { Coins, KeyRound, Plus, Search, UserRound, UsersRound } from 'lucide-react'
import { adjustCredits, getUsers, regenerateKey, updateUser } from '@/api/accounts'
import { extractErrorMessage } from '@/lib/utils'
import type { AuthUser, ManagedUser, Role } from '@/types/accounts'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent } from '@/components/ui/card'
import { Input, formControlClass } from '@/components/ui/input'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { CreateUserDialog } from '@/components/create-user-dialog'
import { KeyDialog } from '@/components/key-dialog'
import { formatCredits } from '@/components/account-dashboard'
import { roleName } from '@/components/admin-shell'
import { EmptyState, PageSection, tableCellClass, tableClass, tableHeadClass, tableRowClass } from '@/components/page-elements'

export function UsersPanel({ currentUser }: { currentUser: AuthUser }) {
  const [search, setSearch] = useState('')
  const [createOpen, setCreateOpen] = useState(false)
  const [creditUser, setCreditUser] = useState<ManagedUser | null>(null)
  const [amount, setAmount] = useState('')
  const [note, setNote] = useState('')
  const [newKey, setNewKey] = useState('')
  const queryClient = useQueryClient()
  const { data: users = [], isLoading } = useQuery({ queryKey: ['users'], queryFn: getUsers })
  const invalidate = () => { queryClient.invalidateQueries({ queryKey: ['users'] }); queryClient.invalidateQueries({ queryKey: ['transactions'] }) }
  const updateMutation = useMutation({ mutationFn: ({ id, enabled, role }: { id: number; enabled?: boolean; role?: Role }) => updateUser(id, { enabled, role }), onSuccess: () => { invalidate(); toast.success('账户状态已更新') }, onError: error => toast.error(extractErrorMessage(error)) })
  const creditsMutation = useMutation({
    mutationFn: ({ id, amount, note }: { id: number; amount: number; note: string }) => adjustCredits(id, amount, note),
    onSuccess: user => { invalidate(); setCreditUser(null); setAmount(''); setNote(''); toast.success(`已调整 ${user.username} 的积分，余额 ${formatCredits(user.balanceMicro)}`) },
    onError: error => toast.error(extractErrorMessage(error)),
  })
  const keyMutation = useMutation({ mutationFn: regenerateKey, onSuccess: result => { setNewKey(result.apiKey); invalidate() }, onError: error => toast.error(extractErrorMessage(error)) })
  const filtered = users.filter(user => user.username.toLowerCase().includes(search.toLowerCase()))

  return (
    <div className="space-y-4">
      <PageSection title="我的账户与代理" description={`${users.length} 个可见账户 · 可创建直属下级代理，管理本人及后代分支`} actions={<><div className="relative"><Search className="pointer-events-none absolute left-3 top-3 h-4 w-4 text-muted-foreground" /><Input aria-label="搜索用户" value={search} onChange={event => setSearch(event.target.value)} placeholder="搜索用户" className="w-full pl-9 sm:w-52" /></div><Button onClick={() => setCreateOpen(true)}><Plus />创建下级账户</Button></>}>
        <Card><CardContent className="p-0"><div className="overflow-x-auto"><table className={tableClass}><thead className={tableHeadClass}><tr><th className="px-5 py-3 font-medium">用户</th><th className="px-4 py-3 font-medium">角色</th><th className="px-4 py-3 text-right font-medium">积分余额</th><th className="px-4 py-3 text-right font-medium">累计消费</th><th className="px-4 py-3 font-medium">状态</th><th className="px-5 py-3 text-right font-medium">操作</th></tr></thead><tbody>
          {filtered.map(user => <tr key={user.id} className={tableRowClass}><td className={tableCellClass}><div className="flex items-center gap-3"><div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground"><UserRound className="h-4 w-4" /></div><div className="min-w-0"><div className="truncate font-medium">{user.username}</div><div className="mt-0.5 font-mono text-xs text-muted-foreground">{user.keyPrefix}…</div></div></div></td><td className="px-4 py-4">{user.id === currentUser.id ? <Badge variant="outline">{roleName(user.role)}</Badge> : user.role === 'super_admin' ? <Badge>{roleName(user.role)}</Badge> : <select aria-label={`修改 ${user.username} 的角色`} className={formControlClass + ' w-28 text-xs'} value={user.role} disabled={currentUser.role !== 'super_admin' && user.role !== 'admin'} onChange={event => updateMutation.mutate({ id: user.id, role: event.target.value as Role })}><option value="user">用户</option><option value="admin">管理员</option></select>}</td><td className="px-4 py-4 text-right font-semibold tabular-nums">{formatCredits(user.balanceMicro)}</td><td className="px-4 py-4 text-right tabular-nums text-muted-foreground">{formatCredits(user.totalSpentMicro)}</td><td className="px-4 py-4"><Badge variant={user.enabled ? 'success' : 'secondary'}>{user.enabled ? '启用' : '已禁用'}</Badge></td><td className="px-5 py-4"><div className="flex justify-end gap-1"><Button variant="outline" size="sm" disabled={user.id === currentUser.id} onClick={() => setCreditUser(user)}><Coins />调整积分</Button><Button variant="ghost" size="icon" title="重置 API Key" aria-label={`重置 ${user.username} 的 API Key`} disabled={keyMutation.isPending} onClick={() => { if (confirm(`确定重置 ${user.username} 的 API Key？旧 Key 将立即失效。`)) keyMutation.mutate(user.id) }}><KeyRound /></Button><Button variant="ghost" size="sm" disabled={user.id === currentUser.id || user.role === 'super_admin'} onClick={() => updateMutation.mutate({ id: user.id, enabled: !user.enabled })}>{user.enabled ? '禁用' : '启用'}</Button></div></td></tr>)}
        </tbody></table>{!isLoading && filtered.length === 0 && <EmptyState icon={UsersRound} title="没有匹配的用户" description={search ? '尝试修改搜索关键词，或创建一个新账户。' : '创建第一个用户，系统将为其生成独立 API Key。'} />}</div></CardContent></Card>
      </PageSection>
      <CreateUserDialog open={createOpen} onOpenChange={setCreateOpen} />
      <Dialog open={Boolean(creditUser)} onOpenChange={open => { if (!open) setCreditUser(null) }}><DialogContent><DialogHeader><DialogTitle>调整 {creditUser?.username} 的积分</DialogTitle><DialogDescription>当前余额 {formatCredits(creditUser?.balanceMicro || 0)}。充值填正数，扣减填负数，所有操作都会记录流水。</DialogDescription></DialogHeader><form onSubmit={event => { event.preventDefault(); const value = Number(amount); if (!Number.isFinite(value) || value === 0) return toast.error('请输入非零积分数量'); if (creditUser) creditsMutation.mutate({ id: creditUser.id, amount: value, note }) }} className="space-y-4"><div className="space-y-2"><label className="text-sm font-medium" htmlFor="credit-amount">调整数量</label><Input id="credit-amount" autoFocus type="number" step="0.000001" value={amount} onChange={event => setAmount(event.target.value)} placeholder="例如 100 或 -10" required /></div><div className="space-y-2"><label className="text-sm font-medium" htmlFor="credit-note">备注（可选）</label><Input id="credit-note" value={note} onChange={event => setNote(event.target.value)} placeholder="充值订单或调整原因" maxLength={200} /></div><DialogFooter><Button type="button" variant="outline" onClick={() => setCreditUser(null)}>取消</Button><Button disabled={creditsMutation.isPending}>{creditsMutation.isPending ? '保存中...' : '确认调整'}</Button></DialogFooter></form></DialogContent></Dialog>
      <KeyDialog apiKey={newKey} onClose={() => setNewKey('')} title="API Key 已重置" />
    </div>
  )
}
