import { Activity, CheckCircle2, Loader2, RotateCcw, Server } from 'lucide-react'
import { LiveRequestsCard } from '@/components/live-requests-card'
import { MetricCard, PageSection } from '@/components/page-elements'
import { Card, CardContent } from '@/components/ui/card'
import { useCredentials, useLoadBalancingMode } from '@/hooks/use-credentials'

export function MonitorPanel() {
  const { data, isLoading, error, refetch } = useCredentials()
  const { data: loadBalancing } = useLoadBalancingMode()
  const disabled = data?.credentials.filter(credential => credential.disabled).length || 0
  const loadBalancingMode = loadBalancing?.mode || 'priority'

  return <div className="space-y-7">
    <LiveRequestsCard />
    <PageSection title="凭据池概况" description="上游连接可用性及当前分配状态，凭据管理请前往“上游凭据”。">
      {isLoading ? <Card><CardContent className="flex min-h-28 items-center justify-center gap-2 text-sm text-muted-foreground"><Loader2 className="h-4 w-4 animate-spin" />正在加载凭据池状态…</CardContent></Card> : error ? <Card><CardContent className="p-5 text-sm text-muted-foreground">凭据池状态加载失败，请稍后重试。<button className="ml-2 min-h-10 text-primary" onClick={() => refetch()}>重新加载</button></CardContent></Card> : <div className="grid gap-4 sm:grid-cols-3"><MetricCard label="凭据总数" value={data?.total || 0} detail={`${disabled} 个已禁用`} icon={Server} /><MetricCard label="可用凭据" value={data?.available || 0} detail="可参与上游请求分配" icon={CheckCircle2} tone="success" /><MetricCard label="当前活跃" value={`#${data?.currentId || '-'}`} detail={loadBalancingMode === 'priority' ? '按优先级顺序使用' : '在可用凭据间均衡分配'} icon={RotateCcw} tone="accent" /></div>}
    </PageSection>
    <div className="flex items-center gap-2 text-xs text-muted-foreground"><Activity className="h-3.5 w-3.5" /><span>实时请求为当前实例内存快照，历史用量请查看“用量统计”。</span></div>
  </div>
}
