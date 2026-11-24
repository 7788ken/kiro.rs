import { useState } from 'react'
import { toast } from 'sonner'
import { Archive, ArchiveRestore, Check, ChevronUp, ChevronDown, Globe, Loader2, RefreshCw, RotateCcw, Trash2, Wallet, X } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { tableCellClass, tableClass, tableHeadClass, tableRowClass } from '@/components/page-elements'
import { ArchiveCredentialDialog } from '@/components/archive-credential-dialog'
import { PurgeCredentialDialog } from '@/components/purge-credential-dialog'
import { ProxyBindingDialog, credentialProxyLabel } from '@/components/proxy-binding-dialog'
import { useArchiveCredential, useForceRefreshToken, useResetFailure, useSetDisabled, useSetPriority, useUnarchiveCredential, usePurgeCredential } from '@/hooks/use-credentials'
import { cn } from '@/lib/utils'
import type { BalanceResponse, CredentialStatusItem } from '@/types/api'

interface CredentialListProps {
  credentials: CredentialStatusItem[]
  onViewBalance: (id: number) => void
  selectedIds: Set<number>
  onToggleSelect: (id: number) => void
  balanceMap: Map<number, BalanceResponse>
  loadingBalanceIds: Set<number>
  /** 是否显示彻底删除（仅超级管理员在已归档视图下） */
  canPurge?: boolean
}

export function CredentialList({ credentials, onViewBalance, selectedIds, onToggleSelect, balanceMap, loadingBalanceIds, canPurge = false }: CredentialListProps) {
  return <div className="overflow-x-auto rounded-xl border bg-card"><table className={`${tableClass} min-w-[1180px]`} aria-label="凭据列表"><thead className={tableHeadClass}><tr><th className="w-12 px-4 py-3"><span className="sr-only">选择</span></th><th className="px-3 py-3">凭据</th><th className="px-3 py-3">状态</th><th className="px-3 py-3">登记人</th><th className="px-3 py-3 text-right">优先级</th><th className="px-3 py-3 text-right">成功 / 失败</th><th className="px-3 py-3">最后调用</th><th className="px-3 py-3 text-right">上游剩余额度</th><th className="px-3 py-3 text-right">累计消耗</th><th className="px-3 py-3 text-center">启用</th><th className="px-4 py-3 text-right">操作</th></tr></thead><tbody>{credentials.map(credential => <CredentialRow key={credential.id} credential={credential} onViewBalance={onViewBalance} selected={selectedIds.has(credential.id)} onToggleSelect={() => onToggleSelect(credential.id)} balance={balanceMap.get(credential.id) || null} loadingBalance={loadingBalanceIds.has(credential.id)} canPurge={canPurge} />)}</tbody></table></div>
}

function CredentialRow({ credential, onViewBalance, selected, onToggleSelect, balance, loadingBalance, canPurge }: { credential: CredentialStatusItem; onViewBalance: (id: number) => void; selected: boolean; onToggleSelect: () => void; balance: BalanceResponse | null; loadingBalance: boolean; canPurge: boolean }) {
  const [editingPriority, setEditingPriority] = useState(false)
  const [priorityValue, setPriorityValue] = useState(String(credential.priority))
  const [showArchiveDialog, setShowArchiveDialog] = useState(false)
  const [showProxyDialog, setShowProxyDialog] = useState(false)
  const [showPurgeDialog, setShowPurgeDialog] = useState(false)
  const setDisabled = useSetDisabled()
  const setPriority = useSetPriority()
  const resetFailure = useResetFailure()
  const archiveCredential = useArchiveCredential()
  const unarchiveCredential = useUnarchiveCredential()
  const purgeCredential = usePurgeCredential()
  const forceRefresh = useForceRefreshToken()
  const archived = credential.archived
  const proxyLabel = credentialProxyLabel(credential)
  const handlePriorityChange = () => {
    const priority = Number(priorityValue)
    if (!Number.isInteger(priority) || priority < 0) return toast.error('优先级必须是非负整数')
    setPriority.mutate({ id: credential.id, priority }, { onSuccess: res => { toast.success(res.message); setEditingPriority(false) }, onError: error => toast.error('操作失败: ' + error.message) })
  }
  const changePriority = (priority: number) => setPriority.mutate({ id: credential.id, priority }, { onSuccess: res => toast.success(res.message), onError: error => toast.error('操作失败: ' + error.message) })
  const handleArchive = (reason: string) => archiveCredential.mutate({ id: credential.id, reason }, { onSuccess: res => { toast.success(res.message); setShowArchiveDialog(false) }, onError: error => toast.error('归档失败: ' + error.message) })
  const handleUnarchive = () => unarchiveCredential.mutate(credential.id, { onSuccess: () => toast.success('已恢复，凭据仍处于禁用状态，请手动启用'), onError: error => toast.error('恢复失败: ' + error.message) })
  const handlePurge = () => purgeCredential.mutate(credential.id, { onSuccess: res => { toast.success(res.message); setShowPurgeDialog(false) }, onError: error => toast.error('彻底删除失败: ' + error.message) })
  const lastUsed = credential.lastUsedAt ? new Date(credential.lastUsedAt).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }) : '从未使用'
  return <>
    <tr className={cn(tableRowClass, credential.isCurrent && 'bg-primary/[0.025]', credential.disabled && 'bg-muted/20')}>
      <td className="px-4 py-3">{!archived && <Checkbox aria-label={`选择凭据 ${credential.id}`} checked={selected} onCheckedChange={onToggleSelect} disabled={!credential.canManage} />}</td>
      <td className={tableCellClass}><div className="max-w-[210px]"><p className="truncate text-sm font-medium" title={credential.email}>{credential.email || `Kiro 凭据 #${credential.id}`}</p><p className="mt-1 truncate font-mono text-[11px] text-muted-foreground" title={credential.maskedApiKey}>ID {credential.id} · {credential.endpoint}{credential.authMethod ? ` · ${credential.authMethod}` : ''}</p>{proxyLabel && <p className="mt-1 truncate text-[11px] text-muted-foreground" title={proxyLabel}>代理：{proxyLabel}</p>}{credential.archiveReason && <p className="mt-1 truncate text-[11px] text-muted-foreground" title={credential.archiveReason}>归档原因：{credential.archiveReason}</p>}</div></td>
      <td className={tableCellClass}><div className="flex flex-col items-start gap-1">{archived ? <Badge variant="secondary">已归档</Badge> : <Badge variant={credential.isCurrent ? 'default' : credential.disabled ? 'secondary' : 'success'}>{credential.isCurrent ? '当前活跃' : credential.disabled ? '已禁用' : '可用'}</Badge>}{!credential.canManage && <span className="text-xs text-muted-foreground">仅查看</span>}</div></td>
      <td className={`${tableCellClass} max-w-[130px]`}><span className="block truncate text-xs" title={credential.registeredByName}>{credential.registeredByName || '历史凭证'}</span></td>
      <td className={`${tableCellClass} text-right`}>{archived ? <span className="px-3 text-sm tabular-nums">{credential.priority}</span> : editingPriority && credential.canManage ? <div className="flex items-center justify-end"><Input aria-label={`编辑凭据 ${credential.id} 的优先级`} type="number" min="0" value={priorityValue} onChange={event => setPriorityValue(event.target.value)} className="h-10 w-[64px] px-2" /><Button variant="ghost" size="icon" aria-label={`保存凭据 ${credential.id} 的优先级`} onClick={handlePriorityChange} disabled={setPriority.isPending}><Check /></Button><Button variant="ghost" size="icon" aria-label={`取消编辑凭据 ${credential.id} 的优先级`} onClick={() => { setEditingPriority(false); setPriorityValue(String(credential.priority)) }}><X /></Button></div> : <Button variant="ghost" size="sm" className="tabular-nums" aria-label={`编辑凭据 ${credential.id} 的优先级`} onClick={() => credential.canManage && setEditingPriority(true)} disabled={!credential.canManage} title={credential.canManage ? '编辑优先级' : '仅可查看'}>{credential.priority}</Button>}</td>
      <td className={`${tableCellClass} whitespace-nowrap text-right text-xs tabular-nums`}>{credential.successCount} / <span className={credential.failureCount ? 'text-destructive' : ''}>{credential.failureCount}</span></td>
      <td className={`${tableCellClass} whitespace-nowrap text-xs text-muted-foreground`}>{lastUsed}</td>
      <td className={`${tableCellClass} whitespace-nowrap text-right text-xs tabular-nums`}>{loadingBalance ? <Loader2 className="ml-auto h-4 w-4 animate-spin text-muted-foreground" /> : balance ? `${balance.remaining.toFixed(2)} / ${balance.usageLimit.toFixed(2)}` : '尚未查询'}</td>
      <td className={`${tableCellClass} whitespace-nowrap text-right text-xs tabular-nums`}>{credential.usedCredits.toFixed(2)}</td>
      <td className={`${tableCellClass} text-center`}>{!archived && <Switch aria-label={`${credential.disabled ? '启用' : '禁用'}凭据 ${credential.id}`} checked={!credential.disabled} onCheckedChange={enabled => setDisabled.mutate({ id: credential.id, disabled: !enabled }, { onSuccess: res => toast.success(res.message), onError: error => toast.error('操作失败: ' + error.message) })} disabled={!credential.canManage || setDisabled.isPending} />}</td>
      <td className="px-4 py-2"><div className="flex items-center justify-end"><Button variant="ghost" size="icon" title="查看额度" aria-label={`查看凭据 ${credential.id} 的额度`} onClick={() => onViewBalance(credential.id)}><Wallet /></Button>{credential.canManage && archived && <Button variant="ghost" size="icon" title="恢复凭据" aria-label={`恢复凭据 ${credential.id}`} onClick={handleUnarchive} disabled={unarchiveCredential.isPending}><ArchiveRestore /></Button>}{canPurge && archived && <Button variant="ghost" size="icon" className="text-destructive hover:bg-destructive/10 hover:text-destructive" title="彻底删除" aria-label={`彻底删除凭据 ${credential.id}`} onClick={() => setShowPurgeDialog(true)} disabled={purgeCredential.isPending}><Trash2 /></Button>}{credential.canManage && !archived && <><Button variant="ghost" size="icon" title={credential.authMethod === 'api_key' ? 'API Key 凭据无需刷新 Token' : '强制刷新 Token'} aria-label={`刷新凭据 ${credential.id} 的 Token`} onClick={() => forceRefresh.mutate(credential.id, { onSuccess: res => toast.success(res.message), onError: error => toast.error('刷新失败: ' + error.message) })} disabled={forceRefresh.isPending || credential.disabled || credential.authMethod === 'api_key'}><RefreshCw className={forceRefresh.isPending ? 'animate-spin' : ''} /></Button><Button variant="ghost" size="icon" title="重置失败次数" aria-label={`重置凭据 ${credential.id} 的失败次数`} onClick={() => resetFailure.mutate(credential.id, { onSuccess: res => toast.success(res.message), onError: error => toast.error('操作失败: ' + error.message) })} disabled={resetFailure.isPending || (credential.failureCount === 0 && credential.refreshFailureCount === 0)}><RotateCcw /></Button><Button variant="ghost" size="icon" title="提高优先级" aria-label={`提高凭据 ${credential.id} 的优先级`} onClick={() => changePriority(Math.max(0, credential.priority - 1))} disabled={setPriority.isPending || credential.priority === 0}><ChevronUp /></Button><Button variant="ghost" size="icon" title="降低优先级" aria-label={`降低凭据 ${credential.id} 的优先级`} onClick={() => changePriority(credential.priority + 1)} disabled={setPriority.isPending}><ChevronDown /></Button><Button variant="ghost" size="icon" title="改绑代理" aria-label={`改绑凭据 ${credential.id} 的代理`} onClick={() => setShowProxyDialog(true)}><Globe /></Button><Button variant="ghost" size="icon" title="归档凭据" aria-label={`归档凭据 ${credential.id}`} onClick={() => setShowArchiveDialog(true)}><Archive /></Button></>}</div></td>
    </tr>
    <ArchiveCredentialDialog credentialId={credential.id} open={showArchiveDialog} onOpenChange={setShowArchiveDialog} onConfirm={handleArchive} pending={archiveCredential.isPending} />
    {canPurge && archived && <PurgeCredentialDialog credentialId={credential.id} open={showPurgeDialog} onOpenChange={setShowPurgeDialog} onConfirm={handlePurge} pending={purgeCredential.isPending} />}
    {credential.canManage && !archived && <ProxyBindingDialog credential={credential} open={showProxyDialog} onOpenChange={setShowProxyDialog} />}
  </>
}
