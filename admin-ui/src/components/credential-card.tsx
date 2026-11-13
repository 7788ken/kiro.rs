import { useState } from 'react'
import { toast } from 'sonner'
import { RefreshCw, ChevronUp, ChevronDown, Wallet, Trash2, Loader2, KeyRound, Server, Check, X } from 'lucide-react'
import { Card, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Switch } from '@/components/ui/switch'
import { Input } from '@/components/ui/input'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import type { CredentialStatusItem, BalanceResponse } from '@/types/api'
import { useSetDisabled, useSetPriority, useResetFailure, useDeleteCredential, useForceRefreshToken } from '@/hooks/use-credentials'
import { cn } from '@/lib/utils'

interface CredentialCardProps {
  credential: CredentialStatusItem
  onViewBalance: (id: number) => void
  selected: boolean
  onToggleSelect: () => void
  balance: BalanceResponse | null
  loadingBalance: boolean
}

function formatLastUsed(lastUsedAt: string | null): string {
  if (!lastUsedAt) return '从未使用'
  const diff = Math.max(0, Date.now() - new Date(lastUsedAt).getTime())
  const minutes = Math.floor(diff / 60000)
  if (minutes < 1) return '刚刚'
  if (minutes < 60) return `${minutes} 分钟前`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} 小时前`
  return `${Math.floor(hours / 24)} 天前`
}

export function CredentialCard({ credential, onViewBalance, selected, onToggleSelect, balance, loadingBalance }: CredentialCardProps) {
  const [editingPriority, setEditingPriority] = useState(false)
  const [priorityValue, setPriorityValue] = useState(String(credential.priority))
  const [showDeleteDialog, setShowDeleteDialog] = useState(false)
  const setDisabled = useSetDisabled()
  const setPriority = useSetPriority()
  const resetFailure = useResetFailure()
  const deleteCredential = useDeleteCredential()
  const forceRefresh = useForceRefreshToken()

  const handleToggleDisabled = () => setDisabled.mutate({ id: credential.id, disabled: !credential.disabled }, { onSuccess: res => toast.success(res.message), onError: err => toast.error('操作失败: ' + (err as Error).message) })
  const handlePriorityChange = () => {
    if (!credential.canManage) return
    const priority = Number(priorityValue)
    if (!Number.isInteger(priority) || priority < 0) return toast.error('优先级必须是非负整数')
    setPriority.mutate({ id: credential.id, priority }, { onSuccess: res => { toast.success(res.message); setEditingPriority(false) }, onError: err => toast.error('操作失败: ' + (err as Error).message) })
  }
  const changePriority = (priority: number) => setPriority.mutate({ id: credential.id, priority }, { onSuccess: res => toast.success(res.message), onError: err => toast.error('操作失败: ' + (err as Error).message) })
  const handleReset = () => resetFailure.mutate(credential.id, { onSuccess: res => toast.success(res.message), onError: err => toast.error('操作失败: ' + (err as Error).message) })
  const handleForceRefresh = () => forceRefresh.mutate(credential.id, { onSuccess: res => toast.success(res.message), onError: err => toast.error('刷新失败: ' + (err as Error).message) })
  const handleDelete = () => {
    if (!credential.disabled) { toast.error('请先禁用凭据再删除'); setShowDeleteDialog(false); return }
    deleteCredential.mutate(credential.id, { onSuccess: res => { toast.success(res.message); setShowDeleteDialog(false) }, onError: err => toast.error('删除失败: ' + (err as Error).message) })
  }
  const authMethod = credential.authMethod === 'api_key' ? 'API Key' : credential.authMethod === 'idc' ? 'IdC' : credential.authMethod === 'social' ? 'Social' : credential.authMethod

  return (
    <>
      <Card className={cn('flex h-full flex-col overflow-hidden transition-[border-color,box-shadow] duration-150 hover:border-border', credential.isCurrent && 'border-primary/35 shadow-[0_0_0_1px_hsl(var(--primary)/0.1)]', credential.disabled && 'bg-muted/20')}>
        <CardContent className="flex flex-1 flex-col p-5">
          <div className="flex items-start gap-3">
            <Checkbox aria-label={`选择凭据 ${credential.id}`} checked={selected} onCheckedChange={onToggleSelect} disabled={!credential.canManage} className="mt-1 before:absolute before:-inset-3 before:content-['']" />
            <div className="min-w-0 flex-1"><div className="flex items-center gap-2"><div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-primary/[0.07] text-primary">{credential.authMethod === 'api_key' ? <KeyRound className="h-4 w-4" /> : <Server className="h-4 w-4" />}</div><div className="min-w-0"><h3 className="truncate text-sm font-semibold" title={credential.email}>{credential.email || `Kiro 凭据 #${credential.id}`}</h3><p className="mt-0.5 font-mono text-[11px] text-muted-foreground">ID {credential.id} · {credential.endpoint}</p></div></div></div>
            <Switch aria-label={`${credential.disabled ? '启用' : '禁用'}凭据 ${credential.id}`} checked={!credential.disabled} onCheckedChange={handleToggleDisabled} disabled={!credential.canManage || setDisabled.isPending} />
          </div>
          <div className="mt-4 flex flex-wrap gap-1.5"><Badge variant={credential.isCurrent ? 'default' : credential.disabled ? 'secondary' : 'success'}>{credential.isCurrent ? '当前活跃' : credential.disabled ? '已禁用' : '可用'}</Badge>{authMethod && <Badge variant="outline">{authMethod}</Badge>}{credential.hasProfileArn && <Badge variant="outline">已配置 Profile</Badge>}{!credential.canManage && <Badge variant="secondary">仅查看</Badge>}{credential.disabledReason && <Badge variant="warning">{credential.disabledReason}</Badge>}</div>
          <p className="mt-3 text-xs text-muted-foreground">登记人：<span className="font-medium text-foreground">{credential.registeredByName || '历史凭证（超管管理）'}</span></p>

          <div className="mt-5 grid grid-cols-2 gap-x-4 gap-y-4 border-t pt-4">
            <Info label="优先级"><div className="flex items-center gap-1">{!credential.canManage ? <span className="flex min-h-10 items-center font-semibold tabular-nums">{credential.priority}</span> : editingPriority ? <><Input aria-label="编辑优先级" type="number" value={priorityValue} onChange={e => setPriorityValue(e.target.value)} className="h-10 w-[68px] px-2" min="0" /><Button size="icon" variant="ghost" aria-label="保存优先级" onClick={handlePriorityChange} disabled={setPriority.isPending}><Check /></Button><Button size="icon" variant="ghost" aria-label="取消编辑" onClick={() => { setEditingPriority(false); setPriorityValue(String(credential.priority)) }}><X /></Button></> : <button className="flex min-h-10 items-center gap-1.5 rounded-md font-semibold tabular-nums hover:text-primary" onClick={() => setEditingPriority(true)} title="点击编辑优先级">{credential.priority}<ChevronUp className="h-3.5 w-3.5 text-muted-foreground" /></button>}</div></Info>
            <Info label="订阅等级"><span className="flex min-h-10 items-center font-medium">{loadingBalance ? <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" /> : balance?.subscriptionTitle || '未查询'}</span></Info>
            <Info label="成功 / 失败"><span className="flex min-h-10 items-center tabular-nums"><span>{credential.successCount}</span><span className="mx-1.5 text-muted-foreground">/</span><span className={credential.failureCount > 0 ? 'text-destructive' : ''}>{credential.failureCount}</span></span></Info>
            <Info label="最后调用"><span className="flex min-h-10 items-center">{formatLastUsed(credential.lastUsedAt)}</span></Info>
          </div>

          <div className="mt-4 rounded-lg bg-muted/45 px-3 py-3"><div className="flex items-center justify-between gap-2 text-xs text-muted-foreground"><span>剩余用量</span>{loadingBalance ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <span className="font-medium text-foreground tabular-nums">{balance ? `${balance.remaining.toFixed(2)} / ${balance.usageLimit.toFixed(2)}` : '尚未查询'}</span>}</div>{balance && <><div className="mt-2 h-1.5 overflow-hidden rounded-full bg-background"><div className="h-full rounded-full bg-primary transition-[width]" style={{ width: `${Math.min(100, Math.max(0, 100 - balance.usagePercentage))}%` }} /></div><p className="mt-1.5 text-right text-[11px] text-muted-foreground tabular-nums">{(100 - balance.usagePercentage).toFixed(1)}% 可用</p></>}</div>
          {(credential.maskedApiKey || credential.hasProxy) && <div className="mt-3 space-y-1.5 text-xs text-muted-foreground">{credential.maskedApiKey && <p className="truncate font-mono">{credential.maskedApiKey}</p>}{credential.hasProxy && <p className="truncate">代理：{credential.proxyUrl}</p>}</div>}

          <div className="mt-auto pt-5"><div className="flex flex-wrap gap-2"><Button size="sm" variant="outline" onClick={() => onViewBalance(credential.id)}><Wallet />查看额度</Button>{credential.canManage && <><Button size="sm" variant="ghost" onClick={handleForceRefresh} disabled={forceRefresh.isPending || credential.disabled || credential.authMethod === 'api_key'} title={credential.authMethod === 'api_key' ? 'API Key 凭据无需刷新 Token' : '强制刷新 Token'}><RefreshCw className={forceRefresh.isPending ? 'animate-spin' : ''} />刷新 Token</Button><Button size="icon" variant="ghost" title="重置失败次数" aria-label={`重置凭据 ${credential.id} 的失败次数`} onClick={handleReset} disabled={resetFailure.isPending || (credential.failureCount === 0 && credential.refreshFailureCount === 0)}><RotateCcwIcon /></Button><Button size="icon" variant="ghost" title="提高优先级" aria-label={`提高凭据 ${credential.id} 的优先级`} onClick={() => changePriority(Math.max(0, credential.priority - 1))} disabled={setPriority.isPending || credential.priority === 0}><ChevronUp /></Button><Button size="icon" variant="ghost" title="降低优先级" aria-label={`降低凭据 ${credential.id} 的优先级`} onClick={() => changePriority(credential.priority + 1)} disabled={setPriority.isPending}><ChevronDown /></Button><Button size="icon" variant="ghost" className="text-destructive hover:bg-destructive/10 hover:text-destructive" title={!credential.disabled ? '需要先禁用凭据才能删除' : '删除凭据'} aria-label={`删除凭据 ${credential.id}`} onClick={() => setShowDeleteDialog(true)} disabled={!credential.disabled}><Trash2 /></Button></>}</div></div>
        </CardContent>
      </Card>

      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}><DialogContent><DialogHeader><DialogTitle>确认删除凭据</DialogTitle><DialogDescription>确定要删除凭据 #{credential.id} 吗？此操作无法撤销。</DialogDescription></DialogHeader><DialogFooter><Button variant="outline" onClick={() => setShowDeleteDialog(false)} disabled={deleteCredential.isPending}>取消</Button><Button variant="destructive" onClick={handleDelete} disabled={deleteCredential.isPending || !credential.disabled}>确认删除</Button></DialogFooter></DialogContent></Dialog>
    </>
  )
}

function Info({ label, children }: { label: string; children: React.ReactNode }) {
  return <div className="min-w-0"><p className="text-xs text-muted-foreground">{label}</p><div className="mt-0.5 text-sm">{children}</div></div>
}

function RotateCcwIcon() {
  return <RefreshCw />
}
