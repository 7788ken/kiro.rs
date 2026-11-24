import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { formControlClass } from '@/components/ui/input'
import { useCheckCredentialEgress, useSetCredentialProxy } from '@/hooks/use-credentials'
import { useProxies } from '@/hooks/use-proxies'
import { extractErrorMessage } from '@/lib/utils'
import type { CredentialStatusItem, ProxyItem, SetCredentialProxyRequest } from '@/types/api'

/** 下拉选项文案：名称 · 出口 IP · 绑定数 */
export function proxyOptionLabel(proxy: ProxyItem) {
  return `${proxy.name} · ${proxy.health.egressIp || '出口 IP 未知'} · 已绑定 ${proxy.boundCount}`
}

/** 凭据列表中代理一栏的展示文案 */
export function credentialProxyLabel(credential: CredentialStatusItem) {
  if (credential.proxyId != null) {
    const name = credential.proxyName || `代理 #${credential.proxyId}（已删除）`
    return `${name} · ${credential.egressIp || '出口 IP 未知'}`
  }
  if (credential.proxyUrl?.toLowerCase() === 'direct') return '直连'
  return credential.proxyUrl || null
}

type BindMode = 'pool' | 'direct' | 'clear'

const modeOptions: { value: BindMode; label: string; hint: string }[] = [
  { value: 'pool', label: '指定代理', hint: '固定绑定所选代理池条目' },
  { value: 'direct', label: '直连（direct）', hint: '不使用任何代理' },
  { value: 'clear', label: '跟随全局代理', hint: '清空凭据级代理设置' },
]

export function ProxyBindingDialog({ credential, open, onOpenChange }: { credential: CredentialStatusItem; open: boolean; onOpenChange: (open: boolean) => void }) {
  const [mode, setMode] = useState<BindMode>('pool')
  const [proxyId, setProxyId] = useState('')
  const proxiesQuery = useProxies(open)
  const setProxy = useSetCredentialProxy()
  const checkEgress = useCheckCredentialEgress()
  const enabledProxies = (proxiesQuery.data?.proxies ?? []).filter(proxy => proxy.enabled)

  useEffect(() => {
    if (!open) return
    const isDirect = credential.proxyUrl?.toLowerCase() === 'direct'
    setMode(isDirect ? 'direct' : 'pool')
    setProxyId(credential.proxyId != null ? String(credential.proxyId) : '')
  }, [open, credential.proxyId, credential.proxyUrl])

  const submit = () => {
    let req: SetCredentialProxyRequest
    if (mode === 'pool') {
      if (!proxyId) return toast.error('请选择要绑定的代理')
      req = { proxyId: Number(proxyId) }
    } else if (mode === 'direct') {
      req = { direct: true }
    } else {
      req = { clear: true }
    }
    setProxy.mutate({ id: credential.id, req }, {
      onSuccess: res => { toast.success(res.message); onOpenChange(false) },
      onError: error => toast.error('改绑失败: ' + extractErrorMessage(error)),
    })
  }

  const handleCheck = () => checkEgress.mutate(credential.id, {
    onSuccess: res => res.health.status === 'healthy'
      ? toast.success(`当前出口 IP：${res.health.egressIp}（${res.proxyUrl ? `经 ${res.proxyUrl}` : '直连'}，${res.health.latencyMs ?? '-'} ms）`)
      : toast.error(`出口检测失败：${res.health.lastError || '未知错误'}`),
    onError: error => toast.error('检测失败: ' + extractErrorMessage(error)),
  })

  const current = credentialProxyLabel(credential)

  return <Dialog open={open} onOpenChange={onOpenChange}>
    <DialogContent>
      <DialogHeader>
        <DialogTitle>改绑代理</DialogTitle>
        <DialogDescription>凭据 #{credential.id} 当前代理：{current || '跟随全局代理'}。改绑后会清除凭据上手动填写的代理地址，下一次请求即使用新出口。</DialogDescription>
      </DialogHeader>
      <div className="space-y-3">
        <div role="radiogroup" aria-label="代理绑定方式" className="grid gap-2">
          {modeOptions.map(option => (
            <label key={option.value} className={`flex cursor-pointer items-start gap-2 rounded-lg border px-3 py-2 text-sm transition-colors ${mode === option.value ? 'border-primary/50 bg-primary/[0.04]' : 'hover:bg-muted/40'}`}>
              <input type="radio" name={`bind-mode-${credential.id}`} value={option.value} checked={mode === option.value} onChange={() => setMode(option.value)} disabled={setProxy.isPending} className="mt-0.5 accent-[hsl(var(--primary))]" />
              <span><span className="font-medium">{option.label}</span><span className="mt-0.5 block text-xs text-muted-foreground">{option.hint}</span></span>
            </label>
          ))}
        </div>
        {mode === 'pool' && <select aria-label="选择代理" className={formControlClass} value={proxyId} onChange={e => setProxyId(e.target.value)} disabled={setProxy.isPending || proxiesQuery.isLoading}>
          <option value="">{proxiesQuery.isLoading ? '正在加载代理…' : enabledProxies.length ? '请选择代理' : '暂无启用的代理'}</option>
          {enabledProxies.map(proxy => <option key={proxy.id} value={proxy.id}>{proxyOptionLabel(proxy)}</option>)}
        </select>}
      </div>
      <DialogFooter>
        <Button variant="ghost" className="sm:mr-auto" onClick={handleCheck} disabled={checkEgress.isPending}>{checkEgress.isPending ? '检测中…' : '检测当前出口 IP'}</Button>
        <Button variant="outline" onClick={() => onOpenChange(false)} disabled={setProxy.isPending}>取消</Button>
        <Button onClick={submit} disabled={setProxy.isPending}>{setProxy.isPending ? '保存中…' : '保存'}</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
}
