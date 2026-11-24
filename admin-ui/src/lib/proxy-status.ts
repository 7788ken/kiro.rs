import type { ProxyStatus } from '@/types/api'

// 代理健康状态对应的 Badge 文案与样式
export const proxyStatusMeta: Record<ProxyStatus, { label: string; variant: 'success' | 'destructive' | 'secondary' }> = {
  healthy: { label: '健康', variant: 'success' },
  unhealthy: { label: '异常', variant: 'destructive' },
  unknown: { label: '未知', variant: 'secondary' },
}
