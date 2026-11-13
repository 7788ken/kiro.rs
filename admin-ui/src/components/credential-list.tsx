import { useState } from 'react'
import { toast } from 'sonner'
import { Check, ChevronUp, ChevronDown, Loader2, RefreshCw, RotateCcw, Trash2, Wallet, X } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { tableCellClass, tableClass, tableHeadClass, tableRowClass } from '@/components/page-elements'
import { useDeleteCredential, useForceRefreshToken, useResetFailure, useSetDisabled, useSetPriority } from '@/hooks/use-credentials'
import { cn } from '@/lib/utils'
import type { BalanceResponse, CredentialStatusItem } from '@/types/api'

interface CredentialListProps {
  credentials: CredentialStatusItem[]
  onViewBalance: (id: number) => void
  selectedIds: Set<number>
  onToggleSelect: (id: number) => void
  balanceMap: Map<number, BalanceResponse>
  loadingBalanceIds: Set<number>
}

export function CredentialList({ credentials, onViewBalance, selectedIds, onToggleSelect, balanceMap, loadingBalanceIds }: CredentialListProps) {
  return <div className="overflow-x-auto rounded-xl border bg-card"><table className={`${tableClass} min-w-[1180px]`} aria-label="凭据列表"><thead className={tableHeadClass}><tr><th className="w-12 px-4 py-3"><span className="sr-only">选择</span></th><th className="px-3 py-3">凭据</th><th className="px-3 py-3">状态</th><th className="px-3 py-3">登记人</th><th className="px-3 py-3 text-right">优先级</th><th className="px-3 py-3 text-right">成功 / 失败</th><th className="px-3 py-3">最后调用</th><th className="px-3 py-3 text-right">剩余用量</th><th className="px-3 py-3 text-center">启用</th><th className="px-4 py-3 text-right">操作</th></tr></thead><tbody>{credentials.map(credential => <CredentialRow key={credential.id} credential={credential} onViewBalance={onViewBalance} selected={selectedIds.has(credential.id)} onToggleSelect={() => onToggleSelect(credential.id)} balance={balanceMap.get(credential.id) || null} loadingBalance={loadingBalanceIds.has(credential.id)} />)}</tbody></table></div>
}

function CredentialRow({ credential, onViewBalance, selected, onToggleSelect, balance, loadingBalance }: { credential: CredentialStatusItem; onViewBalance: (id: number) => void; selected: boolean; onToggleSelect: () => void; balance: BalanceResponse | null; loadingBalance: boolean }) {
  const [editingPriority, setEditingPriority] = useState(false)
  const [priorityValue, setPriorityValue] = useState(String(credential.priority))
  const [showDeleteDialog, setShowDeleteDialog] = useState(false)
  const setDisabled = useSetDisabled()
  const setPriority = useSetPriority()
  const resetFailure = useResetFailure()
  const deleteCredential = useDeleteCredential()
  const forceRefresh = useForceRefreshToken()
  const handlePriorityChange = () => {
    const priority = Number(priorityValue)
    if (!Number.isInteger(priority) || priority < 0) return toast.error('优先级必须是非负整数')
    setPriority.mutate({ id: credential.id, priority }, { onSuccess: res => { toast.success(res.message); setEditingPriority(false) }, onError: error => toast.error('操作失败: ' + error.message) })
  }
  const changePriority = (priority: number) => setPriority.mutate({ id: credential.id, priority }, { onSuccess: res => toast.success(res.message), onError: error => toast.error('操作失败: ' + error.message) })
  const handleDelete = () => deleteCredential.mutate(credential.id, { onSuccess: res => { toast.success(res.message); setShowDeleteDialog(false) }, onError: error => toast.error('删除失败: ' + error.message) })
  const lastUsed = credential.lastUsedAt ? new Date(credential.lastUsedAt).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }) : '从未使用'
  return <>
    <tr className={cn(tableRowClass, credential.isCurrent && 'bg-primary/[0.025]', credential.disabled && 'bg-muted/20')}>
      <td className="px-4 py-3"><Checkbox aria-label={`选择凭据 ${credential.id}`} checked={selected} onCheckedChange={onToggleSelect} disabled={!credential.canManage} /></td>
      <td className={tableCellClass}><div className="max-w-[210px]"><p className="truncate text-sm font-medium" title={credential.email}>{credential.email || `Kiro 凭据 #${credential.id}`}</p><p className="mt-1 truncate font-mono text-[11px] text-muted-foreground" title={credential.maskedApiKey}>ID {credential.id} · {credential.endpoint}{credential.authMethod ? ` · ${credential.authMethod}` : ''}</p></div></td>
      <td className={tableCellClass}><div className="flex flex-col items-start gap-1"><Badge variant={credential.isCurrent ? 'default' : credential.disabled ? 'secondary' : 'success'}>{credential.isCurrent ? '当前活跃' : credential.disabled ? '已禁用' : '可用'}</Badge>{!credential.canManage && <span className="text-xs text-muted-foreground">仅查看</span>}</div></td>
      <td className={`${tableCellClass} max-w-[130px]`}><span className="block truncate text-xs" title={credential.registeredByName}>{credential.registeredByName || '历史凭证'}</span></td>
      <td className={`${tableCellClass} text-right`}>{editingPriority && credential.canManage ? <div className="flex items-center justify-end"><Input aria-label={`编辑凭据 ${credential.id} 的优先级`} type="number" min="0" value={priorityValue} onChange={event => setPriorityValue(event.target.value)} className="h-10 w-[64px] px-2" /><Button variant="ghost" size="icon" aria-label={`保存凭据 ${credential.id} 的优先级`} onClick={handlePriorityChange} disabled={setPriority.isPending}><Check /></Button><Button variant="ghost" size="icon" aria-label={`取消编辑凭据 ${credential.id} 的优先级`} onClick={() => { setEditingPriority(false); setPriorityValue(String(credential.priority)) }}><X /></Button></div> : <Button variant="ghost" size="sm" className="tabular-nums" aria-label={`编辑凭据 ${credential.id} 的优先级`} onClick={() => credential.canManage && setEditingPriority(true)} disabled={!credential.canManage} title={credential.canManage ? '编辑优先级' : '仅可查看'}>{credential.priority}</Button>}</td>
      <td className={`${tableCellClass} whitespace-nowrap text-right text-xs tabular-nums`}>{credential.successCount} / <span className={credential.failureCount ? 'text-destructive' : ''}>{credential.failureCount}</span></td>
      <td className={`${tableCellClass} whitespace-nowrap text-xs text-muted-foreground`}>{lastUsed}</td>
      <td className={`${tableCellClass} whitespace-nowrap text-right text-xs tabular-nums`}>{loadingBalance ? <Loader2 className="ml-auto h-4 w-4 animate-spin text-muted-foreground" /> : balance ? `${balance.remaining.toFixed(2)} / ${balance.usageLimit.toFixed(2)}` : '尚未查询'}</td>
      <td className={`${tableCellClass} text-center`}><Switch aria-label={`${credential.disabled ? '启用' : '禁用'}凭据 ${credential.id}`} checked={!credential.disabled} onCheckedChange={enabled => setDisabled.mutate({ id: credential.id, disabled: !enabled }, { onSuccess: res => toast.success(res.message), onError: error => toast.error('操作失败: ' + error.message) })} disabled={!credential.canManage || setDisabled.isPending} /></td>
      <td className="px-4 py-2"><div className="flex items-center justify-end"><Button variant="ghost" size="icon" title="查看额度" aria-label={`查看凭据 ${credential.id} 的额度`} onClick={() => onViewBalance(credential.id)}><Wallet /></Button>{credential.canManage && <><Button variant="ghost" size="icon" title={credential.authMethod === 'api_key' ? 'API Key 凭据无需刷新 Token' : '强制刷新 Token'} aria-label={`刷新凭据 ${credential.id} 的 Token`} onClick={() => forceRefresh.mutate(credential.id, { onSuccess: res => toast.success(res.message), onError: error => toast.error('刷新失败: ' + error.message) })} disabled={forceRefresh.isPending || credential.disabled || credential.authMethod === 'api_key'}><RefreshCw className={forceRefresh.isPending ? 'animate-spin' : ''} /></Button><Button variant="ghost" size="icon" title="重置失败次数" aria-label={`重置凭据 ${credential.id} 的失败次数`} onClick={() => resetFailure.mutate(credential.id, { onSuccess: res => toast.success(res.message), onError: error => toast.error('操作失败: ' + error.message) })} disabled={resetFailure.isPending || (credential.failureCount === 0 && credential.refreshFailureCount === 0)}><RotateCcw /></Button><Button variant="ghost" size="icon" title="提高优先级" aria-label={`提高凭据 ${credential.id} 的优先级`} onClick={() => changePriority(Math.max(0, credential.priority - 1))} disabled={setPriority.isPending || credential.priority === 0}><ChevronUp /></Button><Button variant="ghost" size="icon" title="降低优先级" aria-label={`降低凭据 ${credential.id} 的优先级`} onClick={() => changePriority(credential.priority + 1)} disabled={setPriority.isPending}><ChevronDown /></Button><Button variant="ghost" size="icon" className="text-destructive hover:bg-destructive/10 hover:text-destructive" title={credential.disabled ? '删除凭据' : '需要先禁用凭据才能删除'} aria-label={`删除凭据 ${credential.id}`} onClick={() => setShowDeleteDialog(true)} disabled={!credential.disabled}><Trash2 /></Button></>}</div></td>
    </tr>
    <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}><DialogContent><DialogHeader><DialogTitle>确认删除凭据</DialogTitle><DialogDescription>确定要删除凭据 #{credential.id} 吗？此操作无法撤销。</DialogDescription></DialogHeader><DialogFooter><Button variant="outline" onClick={() => setShowDeleteDialog(false)} disabled={deleteCredential.isPending}>取消</Button><Button variant="destructive" onClick={handleDelete} disabled={deleteCredential.isPending || !credential.disabled}>确认删除</Button></DialogFooter></DialogContent></Dialog>
  </>
}
