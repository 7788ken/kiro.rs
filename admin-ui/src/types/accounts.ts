export type Role = 'super_admin' | 'admin' | 'user'

export interface AuthUser {
  id: number
  username: string
  role: Role
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
  kind: 'initial' | 'adjustment' | 'charge' | 'refund' | 'release' | 'reserve'
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
}
