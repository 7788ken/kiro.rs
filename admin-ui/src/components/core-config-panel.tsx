import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { RefreshCw, Save, ShieldCheck } from 'lucide-react'
import axios from 'axios'
import { storage } from '@/lib/storage'
import { extractErrorMessage } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import { Input, formControlClass } from '@/components/ui/input'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Switch } from '@/components/ui/switch'
import { PageSection } from '@/components/page-elements'

type CoreConfig = {
  revision: number
  region: string
  authRegion: string | null
  apiRegion: string | null
  kiroVersion: string
  systemVersion: string
  nodeVersion: string
  defaultEndpoint: string
  loadBalancingMode: string
  tlsBackend: 'rustls' | 'native-tls'
  extractThinking: boolean
  proxyUrl: string | null
  proxyUsername: string | null
  proxyPasswordSet: boolean
}

const api = axios.create({ baseURL: '/api/admin' })
api.interceptors.request.use(config => {
  const key = storage.getApiKey()
  if (key) config.headers['x-api-key'] = key
  return config
})

export function CoreConfigPanel() {
  const queryClient = useQueryClient()
  const query = useQuery({ queryKey: ['core-config'], queryFn: async () => (await api.get<CoreConfig>('/config/core')).data })
  const [form, setForm] = useState<CoreConfig | null>(null)
  const [password, setPassword] = useState('')
  const [clearPassword, setClearPassword] = useState(false)
  const [confirmOpen, setConfirmOpen] = useState(false)
  useEffect(() => { if (query.data) setForm(query.data) }, [query.data])
  const mutation = useMutation({
    mutationFn: async (config: CoreConfig) => (await api.put<CoreConfig>('/config/core', { ...config, proxyPassword: password || undefined, clearProxyPassword: clearPassword })).data,
    onSuccess: result => {
      setForm(result)
      setPassword('')
      setClearPassword(false)
      queryClient.invalidateQueries({ queryKey: ['core-config'] })
      toast.success(`配置已保存并热重载，新请求立即生效（版本 ${result.revision}）`)
    },
    onError: error => toast.error(extractErrorMessage(error)),
  })
  if (query.isLoading || !form) return <Card><CardContent className="p-6 text-sm text-muted-foreground">{query.isError ? '核心配置加载失败，请刷新重试。' : '正在加载核心配置…'}</CardContent></Card>
  const update = <K extends keyof CoreConfig>(key: K, value: CoreConfig[K]) => setForm(previous => previous ? { ...previous, [key]: value } : previous)
  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    if (!form.region.trim() || !form.kiroVersion.trim() || !form.systemVersion.trim() || !form.nodeVersion.trim()) return toast.error('Region 与客户端版本标识不能为空')
    setConfirmOpen(true)
  }
  return <PageSection title="核心配置" description="保存后原子写入配置并热重载，新请求使用新配置；进行中的流式响应不受影响。" actions={<div className="flex items-center gap-2"><span className="text-xs tabular-nums text-muted-foreground">运行版本 {form.revision}</span><Button variant="ghost" size="icon" title="重新读取配置" aria-label="重新读取配置" onClick={() => query.refetch()} disabled={query.isFetching}><RefreshCw className={query.isFetching ? 'animate-spin' : ''} /></Button></div>}>
    <form onSubmit={submit} className="space-y-5">
      <Card><CardHeader><CardTitle className="flex items-center gap-2 text-base"><ShieldCheck className="h-4 w-4" />Kiro 上游与客户端</CardTitle><CardDescription>版本与区域会影响后续上游请求；凭据级独立配置仍优先于全局回退值。</CardDescription></CardHeader><CardContent className="grid gap-4 md:grid-cols-2">
        <Field id="core-region" label="默认 Region" value={form.region} onChange={value => update('region', value)} required />
        <Field id="core-kiro-version" label="Kiro 客户端版本" value={form.kiroVersion} onChange={value => update('kiroVersion', value)} required />
        <Field id="core-auth-region" label="Auth Region（留空使用默认 Region）" value={form.authRegion || ''} onChange={value => update('authRegion', value || null)} />
        <Field id="core-api-region" label="API Region（留空使用默认 Region）" value={form.apiRegion || ''} onChange={value => update('apiRegion', value || null)} />
        <Field id="core-system-version" label="系统版本标识" value={form.systemVersion} onChange={value => update('systemVersion', value)} required />
        <Field id="core-node-version" label="Node 版本标识" value={form.nodeVersion} onChange={value => update('nodeVersion', value)} required />
      </CardContent></Card>
      <Card><CardHeader><CardTitle className="text-base">请求分配与协议</CardTitle><CardDescription>修改负载均衡、默认端点和响应解析方式，保存后立即应用于新请求。</CardDescription></CardHeader><CardContent className="grid gap-4 md:grid-cols-2">
        <div className="space-y-2"><label htmlFor="core-endpoint" className="text-sm font-medium">默认端点</label><select id="core-endpoint" className={formControlClass} value={form.defaultEndpoint} onChange={event => update('defaultEndpoint', event.target.value)}><option value="ide">IDE</option></select></div>
        <div className="space-y-2"><label htmlFor="core-balancing" className="text-sm font-medium">负载均衡模式</label><select id="core-balancing" className={formControlClass} value={form.loadBalancingMode} onChange={event => update('loadBalancingMode', event.target.value)}><option value="priority">按优先级</option><option value="balanced">均衡分配</option></select></div>
        <div className="space-y-2"><label htmlFor="core-tls" className="text-sm font-medium">TLS 后端</label><select id="core-tls" className={formControlClass} value={form.tlsBackend} onChange={event => update('tlsBackend', event.target.value as CoreConfig['tlsBackend'])}><option value="rustls">Rustls</option><option value="native-tls">Native TLS</option></select></div>
        <div className="flex min-h-10 items-center justify-between gap-4 rounded-lg bg-muted/35 px-3 py-2 md:mt-7"><div><label htmlFor="core-thinking" className="text-sm font-medium">提取非流式 Thinking</label><p className="mt-0.5 text-xs text-muted-foreground">将标签转换为独立 Thinking 块</p></div><Switch id="core-thinking" checked={form.extractThinking} onCheckedChange={checked => update('extractThinking', checked)} /></div>
      </CardContent></Card>
      <Card><CardHeader><CardTitle className="text-base">全局代理</CardTitle><CardDescription>凭据级代理优先。密钥不会回显；密码留空保留原值，也可选择清除认证。</CardDescription></CardHeader><CardContent className="grid gap-4 md:grid-cols-2">
        <Field id="core-proxy-url" label="代理 URL（留空直连）" value={form.proxyUrl || ''} onChange={value => update('proxyUrl', value || null)} placeholder="http://host:port 或 socks5://host:port" />
        <Field id="core-proxy-username" label="代理用户名（可选）" value={form.proxyUsername || ''} onChange={value => update('proxyUsername', value || null)} />
        <div className="space-y-2"><label htmlFor="core-proxy-password" className="text-sm font-medium">代理密码（可选）</label><Input id="core-proxy-password" type="password" autoComplete="new-password" value={password} onChange={event => setPassword(event.target.value)} placeholder={form.proxyPasswordSet ? '已配置，留空保留' : '留空不设置'} /></div>
        <div className="flex min-h-10 items-center gap-3 md:mt-7"><Switch id="core-clear-password" checked={clearPassword} onCheckedChange={setClearPassword} disabled={!form.proxyPasswordSet} /><label htmlFor="core-clear-password" className="text-sm">清除已有代理认证</label></div>
      </CardContent></Card>
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl bg-muted/35 px-4 py-3"><p className="text-pretty text-xs leading-5 text-muted-foreground">密钥、管理员密码、数据库地址和监听地址不会在此页面展示或修改。若其他管理员已保存新版本，系统会要求先刷新，避免覆盖。</p><Button type="submit" disabled={mutation.isPending}><Save />{mutation.isPending ? '保存中…' : '保存并热重载'}</Button></div>
    </form>
    {confirmOpen && <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/30 p-4 backdrop-blur-[2px]" role="dialog" aria-modal="true" aria-labelledby="core-confirm-title"><Card className="w-full max-w-md"><CardHeader><CardTitle id="core-confirm-title">确认切换核心配置？</CardTitle><CardDescription>保存后立即对新请求生效。Region、Kiro 版本、端点、代理或 TLS 的变化可能影响上游连通性，请确认当前填写内容。</CardDescription></CardHeader><CardContent><div className="mb-4 space-y-1 text-sm"><p>Region：<span className="font-mono">{form.region}</span></p><p>Kiro 版本：<span className="font-mono">{form.kiroVersion}</span></p><p>默认端点：<span className="font-mono">{form.defaultEndpoint}</span></p><p>全局代理：<span className="font-mono">{form.proxyUrl || '直连'}</span></p></div><div className="flex justify-end gap-2"><Button variant="outline" onClick={() => setConfirmOpen(false)}>返回检查</Button><Button onClick={() => { setConfirmOpen(false); mutation.mutate(form) }}>确认保存并切换</Button></div></CardContent></Card></div>}
  </PageSection>
}

function Field({ id, label, value, onChange, required, placeholder }: { id: string; label: string; value: string; onChange: (value: string) => void; required?: boolean; placeholder?: string }) {
  return <div className="space-y-2"><label htmlFor={id} className="text-sm font-medium">{label}</label><Input id={id} value={value} onChange={event => onChange(event.target.value)} required={required} placeholder={placeholder} /></div>
}
