import axios from 'axios'
import { storage } from '@/lib/storage'
import type { AccountOverview, BillingConfig, CreditChange, CreditTransaction, ManagedUser, Role } from '@/types/accounts'

const accountApi = axios.create({ baseURL: '/api' })
accountApi.interceptors.request.use((config) => {
  const key = storage.getApiKey()
  if (key) config.headers['x-api-key'] = key
  return config
})

export async function loginWithPassword(username: string, password: string): Promise<{ sessionToken: string; account: AccountOverview }> {
  const { data } = await accountApi.post('/account/login', { username, password })
  return data
}

export async function logoutAccount(): Promise<void> {
  await accountApi.post('/account/logout')
}

export async function getAccount(): Promise<AccountOverview> {
  const { data } = await accountApi.get<AccountOverview>('/account/me')
  return data
}

export interface TransactionFilters {
  userId?: number
  /** RFC3339 或 YYYY-MM-DD（按 UTC 日界） */
  start?: string
  end?: string
  status?: 'success' | 'failed'
  model?: string
  path?: string
  limit?: number
}

export async function getTransactions(filters: TransactionFilters = {}): Promise<CreditTransaction[]> {
  const params = Object.fromEntries(Object.entries({ limit: 200, ...filters }).filter(([, value]) => value !== undefined && value !== ''))
  const { data } = await accountApi.get<CreditTransaction[]>('/account/transactions', { params })
  return data
}

export async function getTransactionFacets(userId?: number): Promise<{ models: string[]; paths: string[] }> {
  const { data } = await accountApi.get<{ models: string[]; paths: string[] }>('/account/transactions/facets', { params: { userId } })
  return data
}

export async function getCreditChanges(userId?: number): Promise<CreditChange[]> {
  const { data } = await accountApi.get<CreditChange[]>('/account/transactions/adjustments', { params: { userId, limit: 200 } })
  return data
}

export async function getUsers(): Promise<ManagedUser[]> {
  const { data } = await accountApi.get<ManagedUser[]>('/admin/users')
  return data
}

export async function createUser(input: { username: string; password: string; role: Role; initialBalance: number }): Promise<{ user: ManagedUser; apiKey: string }> {
  const { data } = await accountApi.post('/admin/users', input)
  return data
}

export async function updateUser(id: number, input: { enabled?: boolean; role?: Role }): Promise<ManagedUser> {
  const { data } = await accountApi.put(`/admin/users/${id}`, input)
  return data
}

export async function adjustCredits(id: number, amount: number, note: string): Promise<ManagedUser> {
  const { data } = await accountApi.post(`/admin/users/${id}/credits`, { amount, note: note || undefined })
  return data
}

export async function regenerateKey(id: number): Promise<{ apiKey: string; message: string }> {
  const { data } = await accountApi.post(`/admin/users/${id}/key`)
  return data
}

export async function changeOwnKey(): Promise<{ apiKey: string; message: string }> {
  const { data } = await accountApi.post('/account/key')
  return data
}

export async function changePassword(currentPassword: string, newPassword: string): Promise<{ apiKey: string; message: string }> {
  const { data } = await accountApi.post('/account/password', { currentPassword, newPassword })
  return data
}

export async function setBilling(config: BillingConfig): Promise<void> {
  await accountApi.put('/admin/billing', config)
}
