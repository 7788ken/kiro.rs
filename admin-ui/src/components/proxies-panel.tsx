import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { Globe, Pencil, Plus, RefreshCw, Trash2, Activity, Upload } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { EmptyState, PageSection, tableCellClass, tableClass, tableHeadClass, tableRowClass } from '@/components/page-elements'
import { useAddProxy, useCheckAllProxies, useCheckProxy, useDeleteProxy, useProxies, useSetProxyEnabled, useUpdateProxy } from '@/hooks/use-proxies'
import { ProxyImportDialog } from '@/components/proxy-import-dialog'
import { proxyStatusMeta as statusMeta } from '@/lib/proxy-status'
import { extractErrorMessage } from '@/lib/utils'
import type { ProxyItem } from '@/types/api'

function formatTime(value: string | null) {
  if (!value) return '从未检测'
  return new Date(value).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
}

export function ProxiesPanel() {
  const { data, isLoading, isError, refetch, isFetching } = useProxies()
  const checkAll = useCheckAllProxies()
  const [editing, setEditing] = useState<ProxyItem | null>(null)
  const [formOpen, setFormOpen] = useState(false)
  const [deleting, setDeleting] = useState<ProxyItem | null>(null)
  const [importOpen, setImportOpen] = useState(false)
  const proxies = data?.proxies ?? []

  const openAdd = () => { setEditing(null); setFormOpen(true) }
  const openEdit = (proxy: ProxyItem) => { setEditing(proxy); setFormOpen(true) }
  const handleCheckAll = () => checkAll.mutate(undefined, {
    onSuccess: res => {
      const healthy = res.proxies.filter(p => p.enabled && p.health.status === 'healthy').length
      const enabled = res.proxies.filter(p => p.enabled).length
      toast.success(`检测完成：${healthy} / ${enabled} 个启用的代理健康`)
    },
    onError: error => toast.error('检测失败: ' + extractErrorMessage(error)),
  })

  return <PageSection
    title="代理池"
    description="每个凭据固定绑定一个代理（固定出口 IP）。添加凭据时未指定代理会自动分配到绑定数最少的健康代理。后台每 10 分钟自动检测一次。"
    actions={<>
      <Button variant="ghost" size="icon" title="刷新列表" aria-label="刷新代理列表" onClick={() => refetch()} disabled={isFetching}><RefreshCw className={isFetching ? 'animate-spin' : ''} /></Button>
      <Button variant="outline" onClick={handleCheckAll} disabled={checkAll.isPending || proxies.length === 0}><Activity className={checkAll.isPending ? 'animate-pulse' : ''} />{checkAll.isPending ? '检测中…' : '全部检测'}</Button>
      <Button variant="outline" onClick={() => setImportOpen(true)}><Upload />批量导入</Button>
      <Button onClick={openAdd}><Plus />添加代理</Button>
    </>}
  >
    {isLoading ? <div className="rounded-xl border bg-card p-6 text-sm text-muted-foreground">正在加载代理池…</div>
      : isError ? <div className="rounded-xl border bg-card p-6 text-sm text-destructive">代理池加载失败，请刷新重试。</div>
      : proxies.length === 0 ? <div className="rounded-xl border bg-card"><EmptyState icon={Globe} title="还没有出口代理" description="代理池为空时，新凭据沿用全局代理或直连。添加代理并检测通过后，新凭据会自动绑定。" action={<div className="flex gap-2"><Button variant="outline" onClick={() => setImportOpen(true)}><Upload />批量导入</Button><Button onClick={openAdd}><Plus />添加代理</Button></div>} /></div>
      : <div className="overflow-x-auto rounded-xl border bg-card"><table className={`${tableClass} min-w-[1080px]`} aria-label="出口代理列表">
        <thead className={tableHeadClass}><tr>
          <th className="px-5 py-3">名称</th><th className="px-3 py-3">地址</th><th className="px-3 py-3">出口 IP</th><th className="px-3 py-3">状态</th><th className="px-3 py-3 text-right">延迟</th><th className="px-3 py-3">最近检测</th><th className="px-3 py-3 text-right">绑定凭据</th><th className="px-3 py-3 text-center">启用</th><th className="px-4 py-3 text-right">操作</th>
        </tr></thead>
        <tbody>{proxies.map(proxy => <ProxyRow key={proxy.id} proxy={proxy} onEdit={() => openEdit(proxy)} onDelete={() => setDeleting(proxy)} />)}</tbody>
      </table></div>}
    <ProxyFormDialog open={formOpen} onOpenChange={setFormOpen} proxy={editing} />
    <DeleteProxyDialog proxy={deleting} onOpenChange={open => { if (!open) setDeleting(null) }} />
    <ProxyImportDialog open={importOpen} onOpenChange={setImportOpen} />
  </PageSection>
}

function ProxyRow({ proxy, onEdit, onDelete }: { proxy: ProxyItem; onEdit: () => void; onDelete: () => void }) {
  const check = useCheckProxy()
  const setEnabled = useSetProxyEnabled()
  const meta = statusMeta[proxy.health.status] ?? statusMeta.unknown
  const handleCheck = () => check.mutate(proxy.id, {
    onSuccess: res => res.health.status === 'healthy' ? toast.success(`${res.name} 出口 IP：${res.health.egressIp}`) : toast.error(`${res.name} 检测失败：${res.health.lastError || '未知错误'}`),
    onError: error => toast.error('检测失败: ' + extractErrorMessage(error)),
  })
  return <tr className={tableRowClass}>
    <td className={tableCellClass}><div className="max-w-[180px]"><p className="truncate font-medium" title={proxy.name}>{proxy.name}</p><p className="mt-1 truncate text-[11px] text-muted-foreground" title={proxy.note}>#{proxy.id}{proxy.note ? ` · ${proxy.note}` : ''}</p></div></td>
    <td className={`${tableCellClass} max-w-[240px]`}><p className="truncate font-mono text-xs" title={proxy.url}>{proxy.url}</p>{proxy.username && <p className="mt-1 truncate text-[11px] text-muted-foreground">认证：{proxy.username}{proxy.passwordSet ? ' / 已配置密码' : ''}</p>}</td>
    <td className={`${tableCellClass} font-mono text-xs`}>{proxy.health.egressIp || '—'}</td>
    <td className={tableCellClass}><div className="flex flex-col items-start gap-1"><Badge variant={meta.variant}>{meta.label}</Badge>{proxy.health.status === 'unhealthy' && proxy.health.lastError && <span className="max-w-[200px] truncate text-[11px] text-destructive" title={proxy.health.lastError}>{proxy.health.lastError}</span>}</div></td>
    <td className={`${tableCellClass} whitespace-nowrap text-right text-xs tabular-nums`}>{proxy.health.latencyMs != null ? `${proxy.health.latencyMs} ms` : '—'}</td>
    <td className={`${tableCellClass} whitespace-nowrap text-xs text-muted-foreground`}>{formatTime(proxy.health.checkedAt)}</td>
    <td className={`${tableCellClass} text-right tabular-nums`}>{proxy.boundCount}</td>
    <td className={`${tableCellClass} text-center`}><Switch aria-label={`${proxy.enabled ? '停用' : '启用'}代理 ${proxy.name}`} checked={proxy.enabled} disabled={setEnabled.isPending} onCheckedChange={enabled => setEnabled.mutate({ id: proxy.id, enabled }, { onSuccess: () => toast.success(`代理 ${proxy.name} 已${enabled ? '启用' : '停用'}`), onError: error => toast.error('操作失败: ' + extractErrorMessage(error)) })} /></td>
    <td className="px-4 py-2"><div className="flex items-center justify-end">
      <Button variant="ghost" size="icon" title="检测出口 IP" aria-label={`检测代理 ${proxy.name}`} onClick={handleCheck} disabled={check.isPending}><RefreshCw className={check.isPending ? 'animate-spin' : ''} /></Button>
      <Button variant="ghost" size="icon" title="编辑" aria-label={`编辑代理 ${proxy.name}`} onClick={onEdit}><Pencil /></Button>
      <Button variant="ghost" size="icon" title={proxy.boundCount > 0 ? `仍有 ${proxy.boundCount} 个凭据绑定，请先改绑` : '删除'} aria-label={`删除代理 ${proxy.name}`} onClick={onDelete} disabled={proxy.boundCount > 0}><Trash2 /></Button>
    </div></td>
  </tr>
}

function ProxyFormDialog({ open, onOpenChange, proxy }: { open: boolean; onOpenChange: (open: boolean) => void; proxy: ProxyItem | null }) {
  const [name, setName] = useState('')
  const [url, setUrl] = useState('')
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [clearPassword, setClearPassword] = useState(false)
  const [note, setNote] = useState('')
  const addProxy = useAddProxy()
  const updateProxy = useUpdateProxy()
  const pending = addProxy.isPending || updateProxy.isPending
  const isEdit = proxy !== null

  useEffect(() => {
    if (!open) return
    setName(proxy?.name ?? '')
    setUrl(proxy?.url ?? '')
    setUsername(proxy?.username ?? '')
    setPassword('')
    setClearPassword(false)
    setNote(proxy?.note ?? '')
  }, [open, proxy])

  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    if (!name.trim()) return toast.error('请填写代理名称')
    if (!url.trim()) return toast.error('请填写代理 URL')
    const req = {
      name: name.trim(),
      url: url.trim(),
      username: clearPassword ? undefined : username.trim() || undefined,
      password: clearPassword ? undefined : password || undefined,
      clearPassword: clearPassword || undefined,
      note: note.trim() || undefined,
    }
    const options = {
      onSuccess: () => { toast.success(isEdit ? '代理已更新' : '代理已添加，请点击检测确认出口 IP'); onOpenChange(false) },
      onError: (error: unknown) => toast.error((isEdit ? '更新失败: ' : '添加失败: ') + extractErrorMessage(error)),
    }
    if (proxy) updateProxy.mutate({ id: proxy.id, req }, options)
    else addProxy.mutate(req, options)
  }

  return <Dialog open={open} onOpenChange={onOpenChange}>
    <DialogContent>
      <DialogHeader>
        <DialogTitle>{isEdit ? '编辑代理' : '添加代理'}</DialogTitle>
        <DialogDescription>支持 http、https、socks5、socks5h。用户名和密码单独填写，不要写在 URL 里。</DialogDescription>
      </DialogHeader>
      <form onSubmit={submit} className="space-y-4">
        <Field id="proxy-name" label="名称"><Input id="proxy-name" value={name} onChange={e => setName(e.target.value)} placeholder="如：东京-1" maxLength={64} disabled={pending} /></Field>
        <Field id="proxy-url" label="URL"><Input id="proxy-url" value={url} onChange={e => setUrl(e.target.value)} placeholder="http://host:port 或 socks5://host:port" disabled={pending} /></Field>
        <div className="grid gap-4 sm:grid-cols-2">
          <Field id="proxy-username" label="用户名（可选）"><Input id="proxy-username" value={username} onChange={e => setUsername(e.target.value)} autoComplete="off" disabled={pending || clearPassword} /></Field>
          <Field id="proxy-password" label="密码（可选）"><Input id="proxy-password" type="password" autoComplete="new-password" value={password} onChange={e => setPassword(e.target.value)} placeholder={proxy?.passwordSet ? '已配置，留空保留' : '留空不设置'} disabled={pending || clearPassword} /></Field>
        </div>
        {isEdit && <div className="flex items-center gap-3"><Switch id="proxy-clear-password" checked={clearPassword} onCheckedChange={setClearPassword} disabled={!proxy?.passwordSet || pending} /><label htmlFor="proxy-clear-password" className="text-sm">清除密码（同时清除用户名）</label></div>}
        <Field id="proxy-note" label="备注（可选）"><Input id="proxy-note" value={note} onChange={e => setNote(e.target.value)} maxLength={200} disabled={pending} /></Field>
        {isEdit && <p className="text-xs text-muted-foreground">修改地址或认证后健康状态会重置为未知，需要重新检测。已绑定的凭据下一次请求即使用新地址。</p>}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={pending}>取消</Button>
          <Button type="submit" disabled={pending}>{pending ? '保存中…' : '保存'}</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
}

function DeleteProxyDialog({ proxy, onOpenChange }: { proxy: ProxyItem | null; onOpenChange: (open: boolean) => void }) {
  const deleteProxy = useDeleteProxy()
  const confirm = () => {
    if (!proxy) return
    deleteProxy.mutate(proxy.id, {
      onSuccess: res => { toast.success(res.message); onOpenChange(false) },
      onError: error => toast.error('删除失败: ' + extractErrorMessage(error)),
    })
  }
  return <Dialog open={proxy !== null} onOpenChange={onOpenChange}>
    <DialogContent>
      <DialogHeader>
        <DialogTitle>删除代理</DialogTitle>
        <DialogDescription>{proxy && proxy.boundCount > 0 ? `代理「${proxy.name}」仍绑定 ${proxy.boundCount} 个凭据，请先改绑后再删除。` : `确认删除代理「${proxy?.name ?? ''}」？删除后不可恢复。`}</DialogDescription>
      </DialogHeader>
      <DialogFooter>
        <Button variant="outline" onClick={() => onOpenChange(false)} disabled={deleteProxy.isPending}>取消</Button>
        <Button variant="destructive" onClick={confirm} disabled={deleteProxy.isPending || !proxy || proxy.boundCount > 0}>{deleteProxy.isPending ? '删除中…' : '确认删除'}</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
}

function Field({ id, label, children }: { id: string; label: string; children: React.ReactNode }) {
  return <div className="space-y-2"><label htmlFor={id} className="text-sm font-medium">{label}</label>{children}</div>
}
