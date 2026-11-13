import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import axios from 'axios'
import { CalendarDays, CheckCircle2, Coins, TrendingUp, UsersRound } from 'lucide-react'
import { storage } from '@/lib/storage'
import { getUsers } from '@/api/accounts'
import { MetricCard, PageSection } from '@/components/page-elements'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input, formControlClass } from '@/components/ui/input'
import { Badge } from '@/components/ui/badge'
import { formatCredits } from '@/components/account-dashboard'

type UsageGroup = { key: string; requests: number; successfulRequests: number; failedRequests: number; inputTokens: number; outputTokens: number; totalTokens: number; spentMicro: number; averageDurationMs?: number; averageFirstTokenMs?: number }
type UsageStats = { startDate: string; endDate: string; timeZone: string; groupBy: string; totalRequests: number; successfulRequests: number; failedRequests: number; activeUsers: number; inputTokens: number; outputTokens: number; totalTokens: number; spentMicro: number; groups: UsageGroup[] }

const api = axios.create({ baseURL: '/api/admin' })
api.interceptors.request.use(config => {
  const key = storage.getApiKey()
  if (key) config.headers['x-api-key'] = key
  return config
})

function localDate(date: Date) {
  return date.toISOString().slice(0, 10)
}

function beijingDate(offsetDays = 0) {
  return localDate(new Date(Date.now() + (8 * 3600 + offsetDays * 86400) * 1000))
}

export function UsagePanel({ isManager, currentUserId }: { isManager: boolean; currentUserId: number }) {
  const today = beijingDate()
  const [startDate, setStartDate] = useState(today)
  const [endDate, setEndDate] = useState(today)
  const [groupBy, setGroupBy] = useState('day')
  const [userId, setUserId] = useState<number | undefined>(isManager ? undefined : currentUserId)
  const [applied, setApplied] = useState({ startDate, endDate, groupBy, userId })
  const { data: users = [] } = useQuery({ queryKey: ['users'], queryFn: getUsers, enabled: isManager })
  const { data, isLoading, isError, error, refetch, isFetching } = useQuery<UsageStats>({
    queryKey: ['usage-stats', applied],
    queryFn: async () => (await api.get('/monitoring/usage', { params: applied })).data,
    enabled: Boolean(applied.startDate && applied.endDate),
    retry: false,
  })
  const maxTokens = useMemo(() => Math.max(...(data?.groups.map(group => group.totalTokens) || [1]), 1), [data?.groups])
  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    if (startDate > endDate) return
    setApplied({ startDate, endDate, groupBy, userId })
  }
  const groupTitle = applied.groupBy === 'user' ? '按用户' : applied.groupBy === 'model' ? '按模型' : applied.groupBy === 'status' ? '按请求状态' : '按日期'
  return <PageSection title="用量统计" description="按结算时间聚合真实调用流水，Token 与积分消耗不重复计算预扣和返还。" actions={<Button variant="outline" size="sm" onClick={() => refetch()} disabled={isFetching}>刷新数据</Button>}>
    <form onSubmit={submit} className="flex flex-wrap items-end gap-3 rounded-xl bg-muted/35 p-3">
      <div className="space-y-1.5"><label htmlFor="usage-start" className="text-xs font-medium">开始日期</label><Input id="usage-start" type="date" value={startDate} onChange={event => setStartDate(event.target.value)} required /></div>
      <div className="space-y-1.5"><label htmlFor="usage-end" className="text-xs font-medium">结束日期</label><Input id="usage-end" type="date" value={endDate} onChange={event => setEndDate(event.target.value)} required /></div>
      <div className="space-y-1.5"><label htmlFor="usage-group" className="text-xs font-medium">统计维度</label><select id="usage-group" className={formControlClass} value={groupBy} onChange={event => setGroupBy(event.target.value)}><option value="day">按日期</option><option value="user">按用户</option><option value="model">按模型</option><option value="status">按请求状态</option></select></div>
      {isManager && <div className="space-y-1.5"><label htmlFor="usage-user" className="text-xs font-medium">用户</label><select id="usage-user" className={formControlClass} value={userId ?? ''} onChange={event => setUserId(event.target.value ? Number(event.target.value) : undefined)}><option value="">全部用户</option>{users.map(user => <option key={user.id} value={user.id}>{user.username}</option>)}</select></div>}
      <Button type="submit" disabled={startDate > endDate}>查询统计</Button>
      <div className="ml-auto flex flex-wrap gap-1"><Button type="button" variant="ghost" size="sm" onClick={() => { setStartDate(today); setEndDate(today); setApplied({ startDate: today, endDate: today, groupBy, userId }) }}>今天</Button><Button type="button" variant="ghost" size="sm" onClick={() => { const date = beijingDate(-1); setStartDate(date); setEndDate(date); setApplied({ startDate: date, endDate: date, groupBy, userId }) }}>昨天</Button><Button type="button" variant="ghost" size="sm" onClick={() => { const date = beijingDate(-6); setStartDate(date); setEndDate(today); setApplied({ startDate: date, endDate: today, groupBy, userId }) }}>近 7 天</Button></div>
    </form>
    <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
      <MetricCard label="总 Token" value={data?.totalTokens.toLocaleString() ?? '—'} detail={`输入 ${data?.inputTokens.toLocaleString() ?? '—'} · 输出 ${data?.outputTokens.toLocaleString() ?? '—'}`} icon={TrendingUp} tone="accent" />
      <MetricCard label="实际消耗积分" value={data ? formatCredits(data.spentMicro) : '—'} detail="按结算后的实际消费计算" icon={Coins} />
      <MetricCard label="调用次数" value={data?.totalRequests.toLocaleString() ?? '—'} detail={`成功 ${data?.successfulRequests.toLocaleString() ?? '—'} · 失败 ${data?.failedRequests.toLocaleString() ?? '—'}`} icon={CheckCircle2} tone="success" />
      <MetricCard label="活跃用户" value={data?.activeUsers.toLocaleString() ?? '—'} detail="当前筛选范围内的用户数" icon={UsersRound} />
    </div>
    <Card><CardHeader><CardTitle className="text-base">{groupTitle}消耗明细</CardTitle><CardDescription>展示 Token 占比、输入输出拆分、积分消耗及平均耗时。</CardDescription></CardHeader><CardContent className="p-0"><div className="overflow-x-auto"><table className="w-full text-sm"><thead className="border-b bg-muted/30 text-left text-xs text-muted-foreground"><tr><th className="px-4 py-3 font-medium">{groupTitle}</th><th className="px-4 py-3 font-medium">Token 占比</th><th className="px-4 py-3 text-right font-medium">输入 / 输出</th><th className="px-4 py-3 text-right font-medium">积分消耗</th><th className="px-4 py-3 text-right font-medium">调用 / 失败</th><th className="px-4 py-3 text-right font-medium">耗时 / 首字</th></tr></thead><tbody>
      {data?.groups.map(group => <tr key={group.key} className="border-b last:border-0"><td className="max-w-[220px] px-4 py-3"><div className="truncate font-medium" title={group.key}>{group.key}</div><div className="mt-1 text-xs text-muted-foreground">{group.requests.toLocaleString()} 次调用</div></td><td className="min-w-[160px] px-4 py-3"><div className="h-2 overflow-hidden rounded-full bg-muted"><div className="h-full rounded-full bg-primary transition-[width]" style={{ width: `${Math.min(100, group.totalTokens / maxTokens * 100)}%` }} /></div><span className="mt-1 block text-xs tabular-nums text-muted-foreground">{group.totalTokens.toLocaleString()}</span></td><td className="whitespace-nowrap px-4 py-3 text-right text-xs tabular-nums">{group.inputTokens.toLocaleString()}<span className="text-muted-foreground"> / </span>{group.outputTokens.toLocaleString()}</td><td className="px-4 py-3 text-right font-semibold tabular-nums">{formatCredits(group.spentMicro)}</td><td className="whitespace-nowrap px-4 py-3 text-right text-xs tabular-nums">{group.requests.toLocaleString()}<span className="ml-1 text-muted-foreground"> / </span><span className={group.failedRequests ? 'text-destructive' : ''}>{group.failedRequests.toLocaleString()}</span></td><td className="whitespace-nowrap px-4 py-3 text-right text-xs tabular-nums text-muted-foreground">{group.averageDurationMs != null ? `${group.averageDurationMs} ms` : '—'}<span className="block">首字 {group.averageFirstTokenMs != null ? `${group.averageFirstTokenMs} ms` : '—'}</span></td></tr>)}
      {isLoading && <tr><td colSpan={6} className="px-4 py-12 text-center text-sm text-muted-foreground">正在加载用量数据…</td></tr>}
      {isError && <tr><td colSpan={6} className="px-4 py-12 text-center text-sm text-muted-foreground">加载失败，请检查网络后使用“刷新数据”重试。</td></tr>}
      {!isLoading && !isError && !data?.groups.length && <tr><td colSpan={6} className="px-4 py-12 text-center text-sm text-muted-foreground">当前日期和筛选条件下没有调用记录，可调整日期或用户后重试。</td></tr>}
    </tbody></table></div></CardContent></Card>
    {isError && <p role="alert" className="rounded-lg bg-destructive/[0.07] px-4 py-3 text-sm text-destructive">用量数据加载失败：{error instanceof Error ? error.message : '请稍后重试'}</p>}
    <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground"><CalendarDays className="h-3.5 w-3.5" /><span>统计区间：{data?.startDate || applied.startDate} 至 {data?.endDate || applied.endDate}</span><Badge variant="outline">{isFetching ? '正在更新' : '按北京时间（UTC+8）自然日结算'}</Badge></div>
  </PageSection>
}
