import { useQuery } from '@tanstack/react-query'
import { Activity, CheckCircle2, Gauge, TimerReset } from 'lucide-react'
import axios from 'axios'
import { storage } from '@/lib/storage'
import { MetricCard, tableCellClass, tableClass, tableHeadClass, tableRowClass } from '@/components/page-elements'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'


type LiveRequest = {
  requestId: string
  username: string
  model: string
  path: string
  stream: boolean
  startedAt: string
  finishedAt?: string
  httpStatus?: number
  inputTokens: number
  outputTokens: number
  durationMs?: number
  firstTokenMs?: number
  errorMessage?: string
}

type LiveSnapshot = { rpm: number; active: number; recentFinished: number; successRate: number; requests: LiveRequest[]; serverTime: string }

const api = axios.create({ baseURL: '/api/admin' })
api.interceptors.request.use(config => {
  const key = storage.getApiKey()
  if (key) config.headers['x-api-key'] = key
  return config
})

export function LiveRequestsCard() {
  const { data, isLoading, isError, dataUpdatedAt } = useQuery<LiveSnapshot>({ queryKey: ['monitoring-live'], queryFn: async () => (await api.get('/monitoring/live')).data, refetchInterval: 5000, refetchIntervalInBackground: false })
  return <section className="space-y-4">
    <div className="flex flex-wrap items-center justify-between gap-3"><div><h2 className="flex items-center gap-2 text-base font-semibold"><Activity className="h-4 w-4 text-primary" />实时请求</h2><p className="mt-1 text-sm text-muted-foreground">最近一分钟请求速率与当前并发，5 秒自动刷新。</p></div><span className="text-xs tabular-nums text-muted-foreground">{isError ? '刷新失败' : isLoading ? '加载中…' : `更新于 ${new Date(dataUpdatedAt).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit' })}`}</span></div>
    <div className="grid gap-4 sm:grid-cols-3"><MetricCard label="RPM（最近一分钟）" value={data?.rpm ?? '—'} detail="新发起请求数 / 分钟" icon={Gauge} tone="accent" /><MetricCard label="进行中请求" value={data?.active ?? '—'} detail="当前正在处理的 API 调用" icon={TimerReset} /><MetricCard label="最近完成成功率" value={data ? `${(data.successRate * 100).toFixed(1)}%` : '—'} detail={`${data?.recentFinished ?? 0} 个已完成请求`} icon={CheckCircle2} tone="success" /></div>
    <Card><CardHeader className="pb-3"><CardTitle className="text-sm">最近请求</CardTitle><CardDescription>展示当前进行中的调用及最近完成记录，不包含提示词或密钥。</CardDescription></CardHeader><CardContent className="p-0"><div className="max-h-[360px] overflow-auto"><table className={tableClass}><thead className={`sticky top-0 bg-card ${tableHeadClass}`}><tr><th className="px-4 py-2.5 font-medium">状态</th><th className="px-4 py-2.5 font-medium">用户 / 模型</th><th className="px-4 py-2.5 font-medium">开始时间</th><th className="px-4 py-2.5 text-right font-medium">输入 / 输出</th><th className="px-4 py-2.5 text-right font-medium">耗时 / 首字</th></tr></thead><tbody>
      {data?.requests.map(request => <tr key={request.requestId} className={tableRowClass}><td className={tableCellClass}><div className="flex flex-col items-start gap-1.5"><Badge variant={request.finishedAt ? request.errorMessage || request.httpStatus && request.httpStatus >= 400 ? 'destructive' : 'success' : 'warning'}>{request.finishedAt ? request.errorMessage || request.httpStatus && request.httpStatus >= 400 ? '失败' : '完成' : '进行中'}</Badge><span className="text-xs tabular-nums text-muted-foreground">{request.httpStatus ? `HTTP ${request.httpStatus}` : '等待响应'}<span className="ml-1.5">{request.stream ? 'SSE' : '同步'}</span></span></div></td><td className={`${tableCellClass} max-w-[240px]`}><div className="truncate font-medium" title={request.username}>{request.username}</div><div className="mt-0.5 truncate font-mono text-xs text-muted-foreground" title={request.model}>{request.model || '未知模型'}</div></td><td className={`${tableCellClass} whitespace-nowrap text-xs tabular-nums text-muted-foreground`}>{new Date(request.startedAt).toLocaleTimeString('zh-CN')}</td><td className={`${tableCellClass} whitespace-nowrap text-right text-xs tabular-nums`}>{request.inputTokens.toLocaleString()} / {request.outputTokens.toLocaleString()}</td><td className={`${tableCellClass} whitespace-nowrap text-right text-xs tabular-nums text-muted-foreground`}>{request.durationMs !== undefined ? `${request.durationMs} ms` : '—'}<span className="block">首字 {request.firstTokenMs != null ? `${request.firstTokenMs} ms` : '—'}</span></td></tr>)}
      {isLoading && <tr><td colSpan={5} className="px-4 py-10 text-center text-sm text-muted-foreground">正在加载实时请求…</td></tr>}
      {isError && <tr><td colSpan={5} className="px-4 py-10 text-center text-sm text-destructive">实时请求加载失败，将在 5 秒后自动重试。</td></tr>}
      {!isLoading && !isError && !data?.requests.length && <tr><td colSpan={5} className="px-4 py-10 text-center text-sm text-muted-foreground">暂无可展示请求，发起一次 API 调用后即可看到实时数据。</td></tr>}
    </tbody></table></div></CardContent></Card>
  </section>
}
