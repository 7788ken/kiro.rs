import { api } from '@/api/credentials'
import type { ProxyInput, ProxyItem, ProxyListResponse, SuccessResponse } from '@/types/api'

// 获取代理池列表（含健康状态与绑定凭据数）
export async function getProxies(): Promise<ProxyListResponse> {
  const { data } = await api.get<ProxyListResponse>('/proxies')
  return data
}

// 新增代理
export async function addProxy(req: ProxyInput): Promise<ProxyItem> {
  const { data } = await api.post<ProxyItem>('/proxies', req)
  return data
}

// 更新代理（密码留空保留）
export async function updateProxy(id: number, req: ProxyInput): Promise<ProxyItem> {
  const { data } = await api.put<ProxyItem>(`/proxies/${id}`, req)
  return data
}

// 删除代理（有未归档凭据绑定时会被拒绝）
export async function deleteProxy(id: number): Promise<SuccessResponse> {
  const { data } = await api.delete<SuccessResponse>(`/proxies/${id}`)
  return data
}

// 启用或停用代理
export async function setProxyEnabled(id: number, enabled: boolean): Promise<ProxyItem> {
  const { data } = await api.post<ProxyItem>(`/proxies/${id}/enabled`, { enabled })
  return data
}

// 检测单个代理的出口 IP
export async function checkProxy(id: number): Promise<ProxyItem> {
  const { data } = await api.post<ProxyItem>(`/proxies/${id}/check`)
  return data
}

// 检测全部启用的代理
export async function checkAllProxies(): Promise<ProxyListResponse> {
  const { data } = await api.post<ProxyListResponse>('/proxies/check-all')
  return data
}
