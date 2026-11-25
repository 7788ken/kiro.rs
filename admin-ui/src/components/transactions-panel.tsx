import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { ChevronRight, CircleAlert, Coins, ReceiptText, RefreshCw } from 'lucide-react'
import { getCreditChanges, getTransactionFacets, getTransactions, getUsers, type TransactionFilters } from '@/api/accounts'
import type { CreditChange, CreditTransaction, AuthUser } from '@/types/accounts'
import { Card, CardContent } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input, formControlClass } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { formatCredits } from '@/components/account-dashboard'
import { EmptyState, PageSection, tableCellClass, tableClass, tableHeadClass, tableRowClass } from '@/components/page-elements'

type LogStatus = 'success' | 'error' | 'pending'
type CallFilters = { start: string; end: string; status: '' | 'success' | 'failed'; model: string; path: string }

const emptyFilters: CallFilters = { start: '', end: '', status: '', model: '', path: '' }

/** 服务端按 UTC 日界过滤，日期也取 UTC 日历日 */
function utcDate(offsetDays = 0) {
  return new Date(Date.now() + offsetDays * 86400 * 1000).toISOString().slice(0, 10)
}

/** 净输入 = 总输入 − 缓存读 − 缓存写，下限 0 */
function netInputTokens(transaction: CreditTransaction) {
  return Math.max(0, transaction.inputTokens - (transaction.cacheReadInputTokens ?? 0) - (transaction.cacheCreationInputTokens ?? 0))
}

function withCurrent(options: string[], current: string) {
  return current && !options.includes(current) ? [current, ...options] : options
}

export function TransactionsPanel({ isManager, currentUser }: { isManager: boolean; currentUser: AuthUser }) {
  const [userId, setUserId] = useState<number | undefined>()
  const [view, setView] = useState<'calls' | 'adjustments'>('calls')
  const [selected, setSelected] = useState<CreditTransaction | null>(null)
  const [filters, setFilters] = useState<CallFilters>(emptyFilters)
  const [autoRefresh, setAutoRefresh] = useState(false)
  const invalidRange = Boolean(filters.start && filters.end && filters.start > filters.end)
  const callFilters: TransactionFilters = { userId, start: filters.start || undefined, end: filters.end || undefined, status: filters.status || undefined, model: filters.model || undefined, path: filters.path || undefined }
  const refetchInterval = autoRefresh ? 15000 : false
  const { data: users = [] } = useQuery({ queryKey: ['users'], queryFn: getUsers, enabled: isManager })
  const callsQuery = useQuery({ queryKey: ['transactions', userId ?? 'all', filters], queryFn: () => getTransactions(callFilters), enabled: !invalidRange, refetchInterval: view === 'calls' ? refetchInterval : false })
  const changesQuery = useQuery({ queryKey: ['credit-adjustments', userId ?? 'all'], queryFn: () => getCreditChanges(userId), enabled: view === 'adjustments', refetchInterval: view === 'adjustments' ? refetchInterval : false })
  const facetsQuery = useQuery({ queryKey: ['transaction-facets', userId ?? 'all'], queryFn: () => getTransactionFacets(userId), enabled: view === 'calls' })
  const transactions = invalidRange ? [] : callsQuery.data ?? []
  const changes = changesQuery.data ?? []
  const activeQuery = view === 'calls' ? callsQuery : changesQuery
  const hasFilters = Boolean(filters.start || filters.end || filters.status || filters.model || filters.path)
  const update = <K extends keyof CallFilters>(key: K, value: CallFilters[K]) => setFilters(current => ({ ...current, [key]: value }))
  const models = withCurrent(facetsQuery.data?.models ?? [], filters.model)
  const paths = withCurrent(facetsQuery.data?.paths ?? [], filters.path)
  const userFilter = isManager && <div className="space-y-1.5"><label htmlFor="transactions-user" className="text-xs font-medium">用户</label><select id="transactions-user" className={formControlClass + ' sm:w-52'} value={userId ?? ''} onChange={event => setUserId(event.target.value ? Number(event.target.value) : undefined)}><option value="">全部用户</option>{users.map(user => <option value={user.id} key={user.id}>{user.parentId === currentUser.id ? `${user.username}（直属下级）` : user.id === currentUser.id ? `${user.username}（我）` : user.username}</option>)}</select></div>
  const actions = <>
    <div className="flex items-center gap-2"><Switch id="transactions-auto-refresh" checked={autoRefresh} onCheckedChange={setAutoRefresh} /><label htmlFor="transactions-auto-refresh" className="text-sm text-muted-foreground">自动刷新</label></div>
    <Button variant="outline" size="sm" onClick={() => activeQuery.refetch()} disabled={activeQuery.isFetching || (view === 'calls' && invalidRange)}><RefreshCw className={activeQuery.isFetching ? 'animate-spin' : ''} />刷新</Button>
  </>

  return (
    <PageSection title={view === 'calls' ? '调用日志' : '充值与调整'} description={view === 'calls' ? '每次调用按上游凭证的真实消耗结算；没有消耗数据时不扣积分。' : '查看开户初始积分与管理员手工调整，不包含 API 调用。'} actions={actions}>
      <div className="flex gap-1 rounded-lg bg-muted/60 p-1" role="tablist" aria-label="流水类型">
        <button type="button" role="tab" aria-selected={view === 'calls'} onClick={() => setView('calls')} className={`min-h-10 rounded-md px-4 text-sm font-medium transition-[background-color,color,box-shadow] ${view === 'calls' ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`}><ReceiptText className="mr-2 inline-block h-4 w-4 align-[-2px]" />调用日志</button>
        <button type="button" role="tab" aria-selected={view === 'adjustments'} onClick={() => setView('adjustments')} className={`min-h-10 rounded-md px-4 text-sm font-medium transition-[background-color,color,box-shadow] ${view === 'adjustments' ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`}><Coins className="mr-2 inline-block h-4 w-4 align-[-2px]" />充值调整</button>
      </div>
      {(view === 'calls' || isManager) && <div className="flex flex-wrap items-end gap-3 rounded-xl bg-muted/35 p-3">
        {userFilter}
        {view === 'calls' && <>
          <div className="space-y-1.5"><label htmlFor="transactions-start" className="text-xs font-medium">开始日期 <span className="font-normal text-muted-foreground">UTC</span></label><Input id="transactions-start" type="date" value={filters.start} max={filters.end || undefined} onChange={event => update('start', event.target.value)} /></div>
          <div className="space-y-1.5"><label htmlFor="transactions-end" className="text-xs font-medium">结束日期 <span className="font-normal text-muted-foreground">UTC</span></label><Input id="transactions-end" type="date" value={filters.end} min={filters.start || undefined} onChange={event => update('end', event.target.value)} /></div>
          <div className="space-y-1.5"><label htmlFor="transactions-status" className="text-xs font-medium">状态</label><select id="transactions-status" className={formControlClass} value={filters.status} onChange={event => update('status', event.target.value as CallFilters['status'])}><option value="">全部</option><option value="success">成功</option><option value="failed">失败</option></select></div>
          <div className="space-y-1.5"><label htmlFor="transactions-model" className="text-xs font-medium">模型</label><select id="transactions-model" className={formControlClass + ' sm:max-w-56'} value={filters.model} onChange={event => update('model', event.target.value)}><option value="">全部</option>{models.map(model => <option key={model} value={model}>{model}</option>)}</select></div>
          <div className="space-y-1.5"><label htmlFor="transactions-path" className="text-xs font-medium">接口</label><select id="transactions-path" className={formControlClass + ' sm:max-w-56'} value={filters.path} onChange={event => update('path', event.target.value)}><option value="">全部</option>{paths.map(path => <option key={path} value={path}>{path}</option>)}</select></div>
          <div className="ml-auto flex flex-wrap gap-1"><Button type="button" variant="ghost" size="sm" onClick={() => setFilters(current => ({ ...current, start: utcDate(), end: utcDate() }))}>今天</Button><Button type="button" variant="ghost" size="sm" onClick={() => setFilters(current => ({ ...current, start: utcDate(-6), end: utcDate() }))}>近 7 天</Button><Button type="button" variant="ghost" size="sm" disabled={!hasFilters} onClick={() => setFilters(emptyFilters)}>清空筛选</Button></div>
        </>}
      </div>}
      {view === 'calls' && invalidRange && <p role="alert" className="text-sm text-destructive">开始日期不能晚于结束日期。</p>}
      {view === 'calls' ? <>
        <div className="space-y-3 lg:hidden">{transactions.map(transaction => <TransactionCard key={transaction.id} transaction={transaction} isManager={isManager} onSelect={() => setSelected(transaction)} />)}</div>
        <Card className="hidden lg:block"><CardContent className="p-0"><div className="overflow-x-auto"><table className={tableClass}><thead className={tableHeadClass}><tr><th className="px-5 py-3 font-medium">时间 / 状态</th>{isManager && <th className="px-4 py-3 font-medium">用户</th>}<th className="px-4 py-3 font-medium">模型 / 接口</th><th className="px-4 py-3 font-medium">Token（输入(不含缓存) / 输出）</th><th className="px-4 py-3 text-right font-medium">耗时 / 首字</th><th className="px-4 py-3 text-right font-medium">积分变动</th><th className="px-5 py-3 text-right font-medium">变动后余额</th><th className="w-12"><span className="sr-only">查看详情</span></th></tr></thead><tbody>
          {transactions.map(transaction => <TransactionRow key={transaction.id} transaction={transaction} isManager={isManager} onSelect={() => setSelected(transaction)} />)}
        </tbody></table></div></CardContent></Card>
        {!callsQuery.isLoading && !invalidRange && transactions.length === 0 && <Card>{hasFilters ? <EmptyState icon={ReceiptText} title="没有符合筛选条件的记录" description="可调整日期、状态、模型或接口，或点「清空筛选」查看全部记录。" /> : <EmptyState icon={ReceiptText} title="暂无调用日志" description="使用普通用户 API Key 发起请求后，可在这里查看消费、错误及结算明细。" />}</Card>}
      </> : <Card><CardContent className="p-0"><div className="overflow-x-auto"><table className={tableClass}><thead className={tableHeadClass}><tr><th className="px-5 py-3 font-medium">时间 / 类型</th><th className="px-4 py-3 font-medium">备注</th><th className="px-4 py-3 text-right font-medium">积分变动</th><th className="px-5 py-3 text-right font-medium">变动后余额</th></tr></thead><tbody>
        {changes.map((change, index) => <CreditChangeRow key={`${change.createdAt}-${index}`} change={change} />)}
      </tbody></table>{!changesQuery.isLoading && changes.length === 0 && <EmptyState icon={Coins} title="暂无充值或调整" description="开户初始积分与管理员调整会记录在这里，API 调用请查看调用日志。" />}</div></CardContent></Card>}
      {(callsQuery.isError || changesQuery.isError) && <p role="alert" className="rounded-lg bg-destructive/[0.07] px-4 py-3 text-sm text-destructive">流水加载失败，请检查网络后刷新页面重试。</p>}
      <Dialog open={Boolean(selected)} onOpenChange={open => { if (!open) setSelected(null) }}>
        {selected && <TransactionDetail transaction={selected} isManager={isManager} />}
      </Dialog>
    </PageSection>
  )
}

function logStatus(log: CreditTransaction): LogStatus {
  if (log.status === 'pending' || log.status === 'processing') return 'pending'
  return log.errorMessage || log.kind === 'error' ? 'error' : 'success'
}

function statusBadge(status: LogStatus) {
  return <Badge variant={status === 'error' ? 'destructive' : status === 'pending' ? 'warning' : 'success'}>{status === 'error' ? '错误' : status === 'pending' ? '消费中' : '消费'}</Badge>
}

function formatDuration(ms?: number | null) {
  if (ms === undefined || ms === null) return '—'
  return ms >= 1000 ? `${(ms / 1000).toFixed(ms >= 10_000 ? 1 : 2)} s` : `${ms} ms`
}

function TransactionCard({ transaction, isManager, onSelect }: { transaction: CreditTransaction; isManager: boolean; onSelect: () => void }) {
  const status = logStatus(transaction)
  return <Card><CardContent className="p-4"><div className="flex items-start justify-between gap-3"><div className="flex flex-wrap items-center gap-2">{statusBadge(status)}{transaction.httpStatus !== undefined && <span className="text-xs tabular-nums text-muted-foreground">HTTP {transaction.httpStatus}</span>}<span className="text-xs text-muted-foreground">{transaction.stream ? '流式' : '非流式'}</span></div><div className="text-right"><div className="font-semibold tabular-nums">{transaction.amountMicro < 0 ? '−' : ''}{formatCredits(transaction.amountMicro)}</div><div className="mt-1 text-xs text-muted-foreground">积分变动</div></div></div><h3 className="mt-3 break-words text-sm font-medium">{transaction.model || '未知模型'}</h3><p className="mt-1 break-all font-mono text-xs text-muted-foreground">{transaction.path || '/v1/messages'}</p>{transaction.errorMessage && <p className="mt-3 rounded-lg bg-destructive/[0.07] p-3 text-xs leading-5 text-destructive">{transaction.errorMessage}</p>}<dl className="mt-4 grid grid-cols-2 gap-x-3 gap-y-3 text-xs"><div><dt className="text-muted-foreground">Token（输入(不含缓存) / 输出）</dt><dd className="mt-1 tabular-nums">{netInputTokens(transaction).toLocaleString()} / {transaction.outputTokens.toLocaleString()}</dd><CacheTokens transaction={transaction} className="mt-1" /></div><div><dt className="text-muted-foreground">变动后余额</dt><dd className="mt-1 tabular-nums">{formatCredits(transaction.balanceAfterMicro)}</dd></div><div><dt className="text-muted-foreground">耗时 / 首字</dt><dd className="mt-1 tabular-nums">{formatDuration(transaction.durationMs)} / {formatDuration(transaction.firstTokenMs)}</dd></div><div><dt className="text-muted-foreground">{isManager ? '用户' : '调用时间'}</dt><dd className="mt-1 break-words">{isManager ? transaction.username : new Date(transaction.createdAt).toLocaleString('zh-CN')}</dd></div></dl><Button variant="outline" size="sm" className="mt-4 w-full" onClick={onSelect}>查看调用与积分明细<ChevronRight /></Button></CardContent></Card>
}

function TransactionRow({ transaction, isManager, onSelect }: { transaction: CreditTransaction; isManager: boolean; onSelect: () => void }) {
  const status = logStatus(transaction)
  return <tr className={tableRowClass}><td className={tableCellClass}><div className="flex items-center gap-2">{statusBadge(status)}{transaction.httpStatus && <span className="text-xs tabular-nums text-muted-foreground">HTTP {transaction.httpStatus}</span>}</div><div className="mt-1.5 text-xs tabular-nums text-muted-foreground">{new Date(transaction.createdAt).toLocaleString('zh-CN')}</div></td>{isManager && <td className="px-4 py-4">{transaction.username}</td>}<td className="px-4 py-4"><div className="max-w-56"><div className="truncate font-medium" title={transaction.model}>{transaction.model || '未知模型'}</div><div className="mt-1 truncate font-mono text-xs text-muted-foreground" title={transaction.path}>{transaction.path || '/v1/messages'} · {transaction.stream ? '流式' : '非流式'}</div>{transaction.errorMessage && <div className="mt-1 truncate text-xs text-destructive" title={transaction.errorMessage}>{transaction.errorMessage}</div>}</div></td><td className="px-4 py-4 whitespace-nowrap tabular-nums"><div>{netInputTokens(transaction).toLocaleString()}<span className="mx-1.5 text-muted-foreground">/</span>{transaction.outputTokens.toLocaleString()}</div><CacheTokens transaction={transaction} className="mt-1" /></td><td className="px-4 py-4 text-right whitespace-nowrap tabular-nums text-muted-foreground"><div>{formatDuration(transaction.durationMs)}</div><div className="mt-1 text-xs">首字 {formatDuration(transaction.firstTokenMs)}</div></td><td className="px-4 py-4 text-right font-semibold tabular-nums">{transaction.amountMicro < 0 ? '−' : ''}{formatCredits(transaction.amountMicro)}</td><td className="px-5 py-4 text-right tabular-nums text-muted-foreground">{formatCredits(transaction.balanceAfterMicro)}</td><td className="pr-3"><Button variant="ghost" size="icon" aria-label={`查看 ${transaction.model || '请求'} 的调用详情`} title="查看调用详情" onClick={onSelect}><ChevronRight /></Button></td></tr>
}

function TransactionDetail({ transaction, isManager }: { transaction: CreditTransaction; isManager: boolean }) {
  const status = logStatus(transaction)
  const error = status === 'error'
  const pending = status === 'pending'
  return <DialogContent className="max-w-2xl">
    <DialogHeader><div className="mb-2 flex flex-wrap items-center gap-2">{statusBadge(status)}{transaction.httpStatus && <Badge variant="outline">HTTP {transaction.httpStatus}</Badge>}<Badge variant="outline">{transaction.stream ? '流式' : '非流式'}</Badge></div><DialogTitle>{transaction.model || '未知模型'}</DialogTitle><DialogDescription>{new Date(transaction.createdAt).toLocaleString('zh-CN')}{isManager && ` · ${transaction.username}`}</DialogDescription></DialogHeader>
    {error && <section role="alert" className="rounded-xl bg-destructive/[0.07] p-4"><div className="flex items-center gap-2 text-sm font-medium text-destructive"><CircleAlert className="h-4 w-4" />错误详情</div><pre className="mt-3 max-h-44 overflow-auto whitespace-pre-wrap break-words font-sans text-sm leading-6 text-destructive">{transaction.errorMessage || '该历史请求未消费积分。旧版日志未保存具体错误信息。'}</pre><p className="mt-2 text-xs leading-5 text-muted-foreground">{transaction.amountMicro < 0 ? `上游已产生消耗，本次仍扣除 ${formatCredits(transaction.amountMicro)} 积分。` : `本次消费为 0${transaction.refundMicro > 0 ? '，预扣积分已全额返还。' : '，未扣减积分。'}`}</p></section>}
    <section><h3 className="mb-3 text-sm font-semibold">调用信息</h3><dl className="grid gap-x-6 gap-y-4 rounded-xl bg-muted/35 p-4 text-sm sm:grid-cols-2"><Detail label="请求 ID" value={transaction.requestId || transaction.id} mono /><Detail label="接口" value={transaction.path || '历史日志未记录'} mono /><Detail label="输入 Token" value={transaction.inputTokens.toLocaleString()} /><Detail label="净输入 Token" value={netInputTokens(transaction).toLocaleString()} /><Detail label="输出 Token" value={transaction.outputTokens.toLocaleString()} /><Detail label="缓存读取 Token" value={(transaction.cacheReadInputTokens ?? 0).toLocaleString()} /><Detail label="缓存写入 Token" value={(transaction.cacheCreationInputTokens ?? 0).toLocaleString()} /><Detail label="总耗时" value={formatDuration(transaction.durationMs)} /><Detail label="首字时间" value={formatDuration(transaction.firstTokenMs)} /></dl></section>
    <section><div className="mb-3 flex items-center justify-between gap-3"><h3 className="text-sm font-semibold">积分变动</h3><span className="text-xs text-muted-foreground">{pending ? '调用结束后结算' : error && transaction.amountMicro === 0 ? '本次消费 0' : '按凭证消耗 × 消费倍率结算'}</span></div><div className="overflow-hidden rounded-xl bg-muted/35"><BillingStep label="调用前余额" amount={transaction.balanceBeforeMicro} fallback="历史日志未记录" /><BillingStep label="预扣积分" amount={transaction.reservedMicro > 0 ? -transaction.reservedMicro : undefined} balance={transaction.reservedBalanceMicro} fallback={transaction.reservedMicro > 0 ? '余额明细未记录' : '未预扣'} /><BillingStep label={transaction.refundMicro < 0 ? '补扣超额消耗' : error ? '返还预扣' : '返还未使用积分'} amount={transaction.refundMicro || undefined} fallback={pending ? '待结算' : '无返还'} /><BillingStep label={error && transaction.amountMicro === 0 ? '实际消费' : '实际消费（凭证消耗）'} amount={pending ? undefined : transaction.amountMicro} fallback={pending ? '待结算' : undefined} last={!pending} />{pending && <BillingStep label="预扣后余额" amount={undefined} balance={transaction.balanceAfterMicro} last />}{!pending && <div className="flex items-center justify-between gap-4 bg-primary/[0.06] px-4 py-3.5 text-sm"><span className="font-medium">结算后余额</span><span className="font-semibold tabular-nums">{formatCredits(transaction.balanceAfterMicro)}</span></div>}</div>{transaction.note && <p className="mt-3 text-xs leading-5 text-muted-foreground">{transaction.note}</p>}<p className="mt-3 break-all font-mono text-xs leading-5 text-muted-foreground">流水 ID：{transaction.id}</p>{transaction.durationMs === undefined && <p className="mt-2 text-xs leading-5 text-muted-foreground">此记录创建于旧版本，未保存的耗时、首字和完整预扣关联无法补回。</p>}</section>
  </DialogContent>
}

function CacheTokens({ transaction, className = '' }: { transaction: CreditTransaction; className?: string }) {
  const read = transaction.cacheReadInputTokens ?? 0
  const creation = transaction.cacheCreationInputTokens ?? 0
  if (read === 0 && creation === 0) return null
  return <div className={`text-xs tabular-nums text-muted-foreground ${className}`}>缓存读 {read.toLocaleString()} · 缓存写 {creation.toLocaleString()}</div>
}

function Detail({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return <div className="min-w-0"><dt className="text-xs text-muted-foreground">{label}</dt><dd className={`mt-1 break-words tabular-nums ${mono ? 'font-mono text-xs leading-5' : ''}`} title={value}>{value}</dd></div>
}

function BillingStep({ label, amount, balance, fallback, last = false }: { label: string; amount?: number | null; balance?: number | null; fallback?: string; last?: boolean }) {
  return <div className={`flex items-start justify-between gap-4 px-4 py-3 text-sm ${last ? '' : 'border-b border-border/60'}`}><span className="text-muted-foreground">{label}</span><div className="text-right"><span className="font-medium tabular-nums">{amount === undefined || amount === null ? fallback || '—' : `${amount > 0 ? '+' : amount < 0 ? '−' : ''}${formatCredits(amount)}`}</span>{balance !== undefined && balance !== null && <div className="mt-1 text-xs tabular-nums text-muted-foreground">余额 {formatCredits(balance)}</div>}</div></div>
}

function CreditChangeRow({ change }: { change: CreditChange }) {
  const positive = change.amountMicro > 0
  return <tr className={tableRowClass}><td className={tableCellClass}><Badge variant={positive ? 'success' : 'secondary'}>{change.kind === 'initial' ? '初始积分' : '管理员调整'}</Badge><div className="mt-1.5 text-xs tabular-nums text-muted-foreground">{new Date(change.createdAt).toLocaleString('zh-CN')}</div></td><td className="px-4 py-4 text-muted-foreground">{change.note || '—'}</td><td className={`px-4 py-4 text-right font-semibold tabular-nums ${positive ? 'text-success' : ''}`}>{positive ? '+' : change.amountMicro < 0 ? '−' : ''}{formatCredits(change.amountMicro)}</td><td className="px-5 py-4 text-right tabular-nums text-muted-foreground">{formatCredits(change.balanceAfterMicro)}</td></tr>
}
