// 凭据状态响应
export interface CredentialsStatusResponse {
  total: number
  available: number
  currentId: number
  credentials: CredentialStatusItem[]
}

// 单个凭据状态
export interface CredentialStatusItem {
  id: number
  priority: number
  disabled: boolean
  failureCount: number
  isCurrent: boolean
  expiresAt: string | null
  authMethod: string | null
  hasProfileArn: boolean
  email?: string
  refreshTokenHash?: string
  apiKeyHash?: string
  maskedApiKey?: string
  successCount: number
  lastUsedAt: string | null
  usedCredits: number
  hasProxy: boolean
  proxyUrl?: string
  proxyId?: number
  proxyName?: string
  egressIp?: string
  refreshFailureCount: number
  disabledReason?: string
  endpoint: string
  registeredBy?: number
  registeredByName?: string
  canManage: boolean
  archived: boolean
  archivedAt?: string
  archivedBy?: number
  archiveReason?: string
}

// 余额响应
export interface BalanceResponse {
  id: number
  subscriptionTitle: string | null
  currentUsage: number
  usageLimit: number
  remaining: number
  usagePercentage: number
  nextResetAt: number | null
}

// 成功响应
export interface SuccessResponse {
  success: boolean
  message: string
}

// 错误响应
export interface AdminErrorResponse {
  error: {
    type: string
    message: string
  }
}

// 请求类型
export interface SetDisabledRequest {
  disabled: boolean
}

export interface SetPriorityRequest {
  priority: number
}

// 添加凭据请求
export interface AddCredentialRequest {
  refreshToken?: string
  authMethod?: 'social' | 'idc' | 'api_key'
  clientId?: string
  clientSecret?: string
  email?: string
  priority?: number
  profileArn?: string
  authRegion?: string
  apiRegion?: string
  machineId?: string
  proxyUrl?: string
  proxyUsername?: string
  proxyPassword?: string
  proxyId?: number
  kiroApiKey?: string
  endpoint?: string
}

// 修改凭据代理绑定（三选一）
export type SetCredentialProxyRequest = { proxyId: number } | { direct: true } | { clear: true }

// 凭据出口检测结果
export interface CredentialEgressResponse {
  credentialId: number
  proxyUrl?: string
  health: ProxyHealth
}

// ============ 出口代理池 ============

export type ProxyStatus = 'healthy' | 'unhealthy' | 'unknown'

export interface ProxyHealth {
  status: ProxyStatus
  egressIp: string | null
  latencyMs: number | null
  checkedAt: string | null
  lastError: string | null
}

export interface ProxyItem {
  id: number
  name: string
  url: string
  username?: string
  passwordSet: boolean
  note?: string
  enabled: boolean
  createdAt: string
  health: ProxyHealth
  boundCount: number
}

export interface ProxyListResponse {
  proxies: ProxyItem[]
}

// 新增/更新代理：密码留空保留，clearPassword 为 true 时清除
export interface ProxyInput {
  name: string
  url: string
  username?: string
  password?: string
  clearPassword?: boolean
  note?: string
}

// 添加凭据响应
export interface AddCredentialResponse {
  success: boolean
  message: string
  credentialId: number
  email?: string
}
