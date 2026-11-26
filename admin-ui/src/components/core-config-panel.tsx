import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { AlertTriangle, ExternalLink, ImageIcon, RefreshCw, Save, ShieldCheck, SlidersHorizontal, Trash2 } from 'lucide-react'
import axios from 'axios'
import { storage } from '@/lib/storage'
import { extractErrorMessage } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import { Input, formControlClass } from '@/components/ui/input'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Switch } from '@/components/ui/switch'
import { PageSection } from '@/components/page-elements'
import { DEFAULT_SITE_NAME } from '@/api/branding'

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
  siteName: string | null
  logoDataUrl: string | null
  enforceMaxTokens: boolean
  inputTokenSource: InputTokenSource
  promptCacheMode: PromptCacheMode
}

type InputTokenSource = 'upstream' | 'local' | 'min'
type PromptCacheMode = 'simulated' | 'disabled'

const INPUT_TOKEN_SOURCES: { value: InputTokenSource; label: string; hint: string }[] = [
  { value: 'upstream', label: '上游口径', hint: '上游只返回上下文占用百分比，换算值含整段会话的累计占用，同一会话越聊越高' },
  { value: 'local', label: '本地估算', hint: '按本次请求的消息、系统提示和工具定义分词估算，反映单次输入量' },
  { value: 'min', label: '取较小值', hint: '两者取小，兼顾上游真实度与单次输入量级' },
]

const PROMPT_CACHE_MODES: { value: PromptCacheMode; label: string; hint: string }[] = [
  { value: 'simulated', label: '模拟缓存', hint: '按本地断点规则拆出缓存读写，上报的三项之和不变' },
  { value: 'disabled', label: '不模拟', hint: '输入全部计为普通输入，缓存读写恒为 0' },
]

const LOGO_ACCEPT = 'image/svg+xml,image/png,image/jpeg,image/x-icon,image/webp'
const LOGO_MIME_TYPES = LOGO_ACCEPT.split(',')
const MAX_LOGO_BYTES = 200 * 1024
const MAX_SITE_NAME_CHARS = 60

function readAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result))
    reader.onerror = () => reject(reader.error)
    reader.readAsDataURL(file)
  })
}

type VersionStatus = 'ok' | 'behind_latest' | 'unsupported' | 'unknown'

type VersionCheck = {
  configuredVersion: string
  latestVersion: string | null
  latestPubDate: string | null
  minVersion: string
  minSourceUrl: string
  minEffectiveDate: string
  minSummary: string
  checkedAt: string | null
  lastError: string | null
  status: VersionStatus
}

const versionStatusMeta: Record<VersionStatus, { label: string; variant: 'success' | 'secondary' | 'destructive' | 'outline' }> = {
  ok: { label: '正常', variant: 'success' },
  behind_latest: { label: '落后最新版', variant: 'secondary' },
  unsupported: { label: '不受支持', variant: 'destructive' },
  unknown: { label: '未知', variant: 'outline' },
}

function formatCheckedAt(value: string | null) {
  if (!value) return '尚未检测'
  return new Date(value).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
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
  // 新选择的 Logo（未保存）；clearLogo 表示保存时移除已有 Logo
  const [newLogo, setNewLogo] = useState<string | null>(null)
  const [clearLogo, setClearLogo] = useState(false)
  useEffect(() => { if (query.data) setForm(query.data) }, [query.data])
  const mutation = useMutation({
    mutationFn: async (config: CoreConfig) => (await api.put<CoreConfig>('/config/core', {
      ...config,
      siteName: config.siteName ?? '',
      logoDataUrl: clearLogo ? undefined : newLogo ?? undefined,
      clearLogo,
      proxyPassword: password || undefined,
      clearProxyPassword: clearPassword,
    })).data,
    onSuccess: result => {
      setForm(result)
      setPassword('')
      setClearPassword(false)
      setNewLogo(null)
      setClearLogo(false)
      queryClient.invalidateQueries({ queryKey: ['core-config'] })
      queryClient.invalidateQueries({ queryKey: ['branding'] })
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
  const logoPreview = clearLogo ? null : newLogo ?? form.logoDataUrl
  const pickLogo = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    event.target.value = ''
    if (!file) return
    if (!LOGO_MIME_TYPES.includes(file.type)) return toast.error('Logo 仅支持 SVG、PNG、JPEG、ICO、WebP 格式')
    try {
      const dataUrl = await readAsDataUrl(file)
      if (dataUrl.length > MAX_LOGO_BYTES) return toast.error('Logo 不能超过 200 KB，请压缩后再上传')
      setNewLogo(dataUrl)
      setClearLogo(false)
    } catch (error) { toast.error(`读取 Logo 失败：${extractErrorMessage(error)}`) }
  }
  const removeLogo = () => {
    setNewLogo(null)
    setClearLogo(Boolean(form.logoDataUrl))
  }
  return <PageSection title="核心配置" description="保存后原子写入配置并热重载，新请求使用新配置；进行中的流式响应不受影响。" actions={<div className="flex items-center gap-2"><span className="text-xs tabular-nums text-muted-foreground">运行版本 {form.revision}</span><Button variant="ghost" size="icon" title="重新读取配置" aria-label="重新读取配置" onClick={() => query.refetch()} disabled={query.isFetching}><RefreshCw className={query.isFetching ? 'animate-spin' : ''} /></Button></div>}>
    <form onSubmit={submit} className="space-y-5">
      <Card><CardHeader><CardTitle className="flex items-center gap-2 text-base"><ImageIcon className="h-4 w-4" />品牌</CardTitle><CardDescription>站点名称和 Logo 显示在后台页头、登录页和浏览器标题，保存后立即生效。</CardDescription></CardHeader><CardContent className="grid gap-4 md:grid-cols-2">
        <div className="space-y-2"><label htmlFor="core-site-name" className="text-sm font-medium">站点名称（留空使用默认）</label><Input id="core-site-name" value={form.siteName || ''} onChange={event => update('siteName', event.target.value || null)} placeholder={DEFAULT_SITE_NAME} maxLength={MAX_SITE_NAME_CHARS} /></div>
        <div className="space-y-2"><label htmlFor="core-logo" className="text-sm font-medium">Logo（SVG / PNG / JPEG / ICO / WebP，不超过 200 KB）</label><div className="flex items-center gap-3">{logoPreview ? <img src={logoPreview} alt="Logo 预览" className="h-16 w-16 shrink-0 rounded-xl border object-contain" /> : <div className="flex h-16 w-16 shrink-0 items-center justify-center rounded-xl border border-dashed text-xs text-muted-foreground">默认</div>}<div className="flex flex-wrap gap-2"><Input id="core-logo" type="file" accept={LOGO_ACCEPT} onChange={pickLogo} className="max-w-[240px]" />{logoPreview && <Button type="button" variant="outline" size="sm" onClick={removeLogo}><Trash2 />移除 Logo</Button>}</div></div>{clearLogo && <p className="text-xs text-muted-foreground">保存后将移除已有 Logo。</p>}</div>
      </CardContent></Card>
      <Card><CardHeader><CardTitle className="flex items-center gap-2 text-base"><ShieldCheck className="h-4 w-4" />Kiro 上游与客户端</CardTitle><CardDescription>版本与区域会影响后续上游请求；凭据级独立配置仍优先于全局回退值。</CardDescription></CardHeader><CardContent className="grid gap-4 md:grid-cols-2">
        <VersionCheckStatus onUseMinVersion={version => update('kiroVersion', version)} />
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
      <Card><CardHeader><CardTitle className="flex items-center gap-2 text-base"><SlidersHorizontal className="h-4 w-4" />客户端兼容</CardTitle><CardDescription>只改变上报给客户端的用量口径和输出长度约束，不影响按凭证真实消耗扣费。开关在请求开始时锁定，保存不影响在途请求。</CardDescription></CardHeader><CardContent className="grid gap-4 md:grid-cols-2">
        <div className="flex min-h-10 items-start justify-between gap-4 rounded-lg bg-muted/35 px-3 py-2 md:col-span-2"><div><label htmlFor="core-enforce-max-tokens" className="text-sm font-medium">本地强制 max_tokens</label><p className="mt-0.5 text-pretty text-xs text-muted-foreground">上游 Kiro 协议没有 max_tokens 字段，客户端传的值默认只用于预扣，不约束生成。开启后输出达到该上限即停止转发后续内容，并把 stop_reason 置为 max_tokens；工具调用块整块保留，避免产生不可用的调用。</p></div><Switch id="core-enforce-max-tokens" checked={form.enforceMaxTokens} onCheckedChange={checked => update('enforceMaxTokens', checked)} /></div>
        <div className="space-y-2"><label htmlFor="core-input-source" className="text-sm font-medium">输入 Token 口径</label><select id="core-input-source" className={formControlClass} value={form.inputTokenSource} onChange={event => update('inputTokenSource', event.target.value as InputTokenSource)}>{INPUT_TOKEN_SOURCES.map(item => <option key={item.value} value={item.value}>{item.label}</option>)}</select><p className="text-pretty text-xs text-muted-foreground">{INPUT_TOKEN_SOURCES.find(item => item.value === form.inputTokenSource)?.hint}</p></div>
        <div className="space-y-2"><label htmlFor="core-cache-mode" className="text-sm font-medium">Prompt Cache 模式</label><select id="core-cache-mode" className={formControlClass} value={form.promptCacheMode} onChange={event => update('promptCacheMode', event.target.value as PromptCacheMode)}>{PROMPT_CACHE_MODES.map(item => <option key={item.value} value={item.value}>{item.label}</option>)}</select><p className="text-pretty text-xs text-muted-foreground">{PROMPT_CACHE_MODES.find(item => item.value === form.promptCacheMode)?.hint}</p></div>
      </CardContent></Card>
      <Card><CardHeader><CardTitle className="text-base">全局代理</CardTitle><CardDescription>凭据级代理优先。密钥不会回显；密码留空保留原值，也可选择清除认证。</CardDescription></CardHeader><CardContent className="grid gap-4 md:grid-cols-2">
        <Field id="core-proxy-url" label="代理 URL（留空直连）" value={form.proxyUrl || ''} onChange={value => update('proxyUrl', value || null)} placeholder="http://host:port 或 socks5://host:port" />
        <Field id="core-proxy-username" label="代理用户名（可选）" value={form.proxyUsername || ''} onChange={value => update('proxyUsername', value || null)} />
        <div className="space-y-2"><label htmlFor="core-proxy-password" className="text-sm font-medium">代理密码（可选）</label><Input id="core-proxy-password" type="password" autoComplete="new-password" value={password} onChange={event => setPassword(event.target.value)} placeholder={form.proxyPasswordSet ? '已配置，留空保留' : '留空不设置'} /></div>
        <div className="flex min-h-10 items-center gap-3 md:mt-7"><Switch id="core-clear-password" checked={clearPassword} onCheckedChange={setClearPassword} disabled={!form.proxyPasswordSet} /><label htmlFor="core-clear-password" className="text-sm">清除已有代理认证</label></div>
      </CardContent></Card>
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl bg-muted/35 px-4 py-3"><p className="text-pretty text-xs leading-5 text-muted-foreground">密钥、管理员密码、数据库地址和监听地址不会在此页面展示或修改。若其他管理员已保存新版本，系统会要求先刷新，避免覆盖。</p><Button type="submit" disabled={mutation.isPending}><Save />{mutation.isPending ? '保存中…' : '保存并热重载'}</Button></div>
    </form>
    {confirmOpen && <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/30 p-4 backdrop-blur-[2px]" role="dialog" aria-modal="true" aria-labelledby="core-confirm-title"><Card className="w-full max-w-md"><CardHeader><CardTitle id="core-confirm-title">确认切换核心配置？</CardTitle><CardDescription>保存后立即对新请求生效。Region、Kiro 版本、端点、代理或 TLS 的变化可能影响上游连通性，请确认当前填写内容。</CardDescription></CardHeader><CardContent><div className="mb-4 space-y-1 text-sm"><p>站点名称：<span className="font-mono">{form.siteName?.trim() || DEFAULT_SITE_NAME}</span></p><p>Region：<span className="font-mono">{form.region}</span></p><p>Kiro 版本：<span className="font-mono">{form.kiroVersion}</span></p><p>默认端点：<span className="font-mono">{form.defaultEndpoint}</span></p><p>全局代理：<span className="font-mono">{form.proxyUrl || '直连'}</span></p><p>强制 max_tokens：<span className="font-mono">{form.enforceMaxTokens ? '开启' : '关闭'}</span></p><p>输入 Token 口径：<span className="font-mono">{INPUT_TOKEN_SOURCES.find(item => item.value === form.inputTokenSource)?.label}</span></p><p>Prompt Cache：<span className="font-mono">{PROMPT_CACHE_MODES.find(item => item.value === form.promptCacheMode)?.label}</span></p></div><div className="flex justify-end gap-2"><Button variant="outline" onClick={() => setConfirmOpen(false)}>返回检查</Button><Button onClick={() => { setConfirmOpen(false); mutation.mutate(form) }}>确认保存并切换</Button></div></CardContent></Card></div>}
  </PageSection>
}

function Field({ id, label, value, onChange, required, placeholder }: { id: string; label: string; value: string; onChange: (value: string) => void; required?: boolean; placeholder?: string }) {
  return <div className="space-y-2"><label htmlFor={id} className="text-sm font-medium">{label}</label><Input id={id} value={value} onChange={event => onChange(event.target.value)} required={required} placeholder={placeholder} /></div>
}

function VersionCheckStatus({ onUseMinVersion }: { onUseMinVersion: (version: string) => void }) {
  const queryClient = useQueryClient()
  const query = useQuery({ queryKey: ['version-check'], queryFn: async () => (await api.get<VersionCheck>('/config/version-check')).data })
  const refresh = useMutation({
    mutationFn: async () => (await api.post<VersionCheck & { throttled: boolean }>('/config/version-check/refresh')).data,
    onSuccess: result => {
      queryClient.setQueryData(['version-check'], result)
      if (result.throttled) toast.info('60 秒内已检测过，显示的是最近一次结果')
      else if (result.lastError) toast.error('检测已完成，但有错误，请查看状态区说明')
      else toast.success('版本检测完成')
    },
    onError: error => toast.error(extractErrorMessage(error)),
  })
  const data = query.data
  const unsupported = data?.status === 'unsupported'
  return <div className={`space-y-3 rounded-lg border px-4 py-3 md:col-span-2 ${unsupported ? 'border-destructive/30 bg-destructive/[0.05]' : 'bg-muted/35'}`}>
    <div className="flex flex-wrap items-center justify-between gap-2">
      <div className="flex items-center gap-2"><span className="text-sm font-medium">客户端版本检测</span>{data && <Badge variant={versionStatusMeta[data.status].variant}>{unsupported && <AlertTriangle className="h-3 w-3" />}{versionStatusMeta[data.status].label}</Badge>}</div>
      <Button type="button" variant="outline" size="sm" onClick={() => refresh.mutate()} disabled={refresh.isPending || query.isLoading}><RefreshCw className={refresh.isPending ? 'animate-spin' : ''} />{refresh.isPending ? '检测中…' : '立即检测'}</Button>
    </div>
    {query.isLoading ? <p className="text-sm text-muted-foreground">正在读取版本检测结果…</p>
      : query.isError || !data ? <p className="text-sm text-destructive">版本检测结果加载失败，请稍后重试。</p>
      : <>
        <dl className="grid gap-x-6 gap-y-1.5 text-sm sm:grid-cols-2">
          <div className="flex gap-2"><dt className="text-muted-foreground">当前配置版本</dt><dd className="font-mono tabular-nums">{data.configuredVersion}</dd></div>
          <div className="flex flex-wrap gap-x-2"><dt className="text-muted-foreground">官方最新版</dt><dd className="tabular-nums"><span className="font-mono">{data.latestVersion || '未获取'}</span>{data.latestPubDate && <span className="text-muted-foreground">（{data.latestPubDate} 发布）</span>}</dd></div>
          <div className="flex min-w-0 gap-2"><dt className="shrink-0 text-muted-foreground">最低支持版本</dt><dd className="flex min-w-0 items-center gap-1.5 tabular-nums"><span className="font-mono">{data.minVersion}</span><a href={data.minSourceUrl} target="_blank" rel="noreferrer noopener" className="inline-flex items-center gap-0.5 text-primary hover:underline" title={data.minSummary}>官方公告<ExternalLink className="h-3 w-3" /></a></dd></div>
          <div className="flex gap-2"><dt className="text-muted-foreground">检测时间</dt><dd className="tabular-nums">{formatCheckedAt(data.checkedAt)}</dd></div>
        </dl>
        {unsupported && <div className="space-y-2 text-sm text-destructive">
          <p className="text-pretty">当前版本不受官方支持，上游会以 400 拒绝所有对话请求。请把 Kiro 客户端版本改为不低于 {data.minVersion} 后保存。</p>
          <div className="flex flex-wrap items-center gap-3"><Button type="button" variant="destructive" size="sm" onClick={() => { onUseMinVersion(data.minVersion); toast.info(`已填入 ${data.minVersion}，请检查后点击「保存并热重载」`) }}>填入官方最低支持版本</Button><span className="text-pretty text-xs text-muted-foreground">来源：{data.minSummary}。只修改输入框，不会自动保存。</span></div>
        </div>}
        {data.status === 'behind_latest' && <p className="text-pretty text-xs text-muted-foreground">官方已发布更新版本，当前版本仍受支持，可继续使用；未经兼容性验证不建议跟随最新版。</p>}
        {data.lastError && <p className="break-words text-xs text-muted-foreground">最近错误：{data.lastError}</p>}
      </>}
  </div>
}
