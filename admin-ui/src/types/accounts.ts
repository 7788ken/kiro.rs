export type Role = 'super_admin' | 'admin' | 'user'

export interface AuthUser {
  id: number
  username: string
  role: Role
  parentId?: number
  balanceMicro: number
  enabled: boolean
}

export interface BillingConfig {
  inputPerMillion: number
  outputPerMillion: number
}

export interface AccountOverview {
  user: AuthUser
  totalInputTokens: number
  totalOutputTokens: number
  totalSpentMicro: number
  pendingCount: number
  billing: BillingConfig
}

export interface ManagedUser extends AuthUser {
  keyPrefix: string
  createdAt: string
  totalInputTokens: number
  totalOutputTokens: number
  totalSpentMicro: number
}

export interface CreditTransaction {
  id: string
  userId: number
  username: string
  kind: 'reserve' | 'charge' | 'error'
  amountMicro: number
  balanceAfterMicro: number
  model?: string
  inputTokens: number
  outputTokens: number
  reservedMicro: number
  status: 'pending' | 'settled' | 'processing'
  note?: string
  createdAt: string
  completedAt?: string
  requestId?: string
  path?: string
  stream: boolean
  balanceBeforeMicro?: number
  reservedBalanceMicro?: number
  refundMicro: number
  httpStatus?: number
  errorMessage?: string
  durationMs?: number
  firstTokenMs?: number
}

export interface CreditChange {
  kind: 'initial' | 'adjustment'
  amountMicro: number
  balanceAfterMicro: number
  note?: string
  createdAt: string
  completedAt?: string
}
