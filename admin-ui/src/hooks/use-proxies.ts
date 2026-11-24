import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { addProxy, checkAllProxies, checkProxy, deleteProxy, getProxies, setProxyEnabled, updateProxy } from '@/api/proxies'
import type { ProxyInput } from '@/types/api'

// 查询代理池列表
export function useProxies(enabled = true) {
  return useQuery({
    queryKey: ['proxies'],
    queryFn: getProxies,
    enabled,
    refetchInterval: 60000,
  })
}

function useInvalidateProxies() {
  const queryClient = useQueryClient()
  return () => {
    queryClient.invalidateQueries({ queryKey: ['proxies'] })
    queryClient.invalidateQueries({ queryKey: ['credentials'] })
  }
}

// 新增代理
export function useAddProxy() {
  const invalidate = useInvalidateProxies()
  return useMutation({ mutationFn: (req: ProxyInput) => addProxy(req), onSuccess: invalidate })
}

// 更新代理
export function useUpdateProxy() {
  const invalidate = useInvalidateProxies()
  return useMutation({ mutationFn: ({ id, req }: { id: number; req: ProxyInput }) => updateProxy(id, req), onSuccess: invalidate })
}

// 删除代理
export function useDeleteProxy() {
  const invalidate = useInvalidateProxies()
  return useMutation({ mutationFn: (id: number) => deleteProxy(id), onSuccess: invalidate })
}

// 启用或停用代理
export function useSetProxyEnabled() {
  const invalidate = useInvalidateProxies()
  return useMutation({ mutationFn: ({ id, enabled }: { id: number; enabled: boolean }) => setProxyEnabled(id, enabled), onSuccess: invalidate })
}

// 检测单个代理
export function useCheckProxy() {
  const invalidate = useInvalidateProxies()
  return useMutation({ mutationFn: (id: number) => checkProxy(id), onSuccess: invalidate })
}

// 检测全部代理
export function useCheckAllProxies() {
  const invalidate = useInvalidateProxies()
  return useMutation({ mutationFn: checkAllProxies, onSuccess: invalidate })
}
