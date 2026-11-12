import { useEffect, useState } from 'react'
import { ReceiptText, Settings2, ShieldCheck, UsersRound, Wallet, type LucideIcon } from 'lucide-react'

export const adminSections = ['credentials', 'users', 'overview', 'transactions', 'settings'] as const
export type AdminSection = typeof adminSections[number]

export const sectionMeta: Record<AdminSection, { title: string; description: string }> = {
  credentials: { title: '上游凭据', description: '管理 Kiro 连接池、订阅额度与请求分配策略。' },
  users: { title: '用户与额度', description: '创建独立账户、分配 API Key 并管理积分额度。' },
  overview: { title: '我的账户', description: '查看账户余额、Token 用量与当前计费标准。' },
  transactions: { title: '积分流水', description: '查看每次调用的消费与错误状态，追踪首字、Token、预扣、返还及余额。' },
  settings: { title: '安全与设置', description: '管理登录密码、API Key 与平台计费规则。' },
}

export const sectionNavItems = [
  { id: 'credentials', label: '上游凭据', icon: ShieldCheck, requiresSuperAdmin: true },
  { id: 'users', label: '用户与额度', icon: UsersRound, requiresManager: true },
  { id: 'overview', label: '我的账户', icon: Wallet },
  { id: 'transactions', label: '积分流水', icon: ReceiptText },
  { id: 'settings', label: '安全与设置', icon: Settings2 },
] satisfies { id: AdminSection; label: string; icon: LucideIcon; requiresSuperAdmin?: boolean; requiresManager?: boolean }[]

export function sectionPath(section: AdminSection) {
  return `/admin/${section}`
}

function sectionFromPath(pathname: string): AdminSection | null {
  const normalized = pathname.replace(/\/+$/, '') || '/'
  const section = normalized.split('/')[2]
  return adminSections.includes(section as AdminSection) ? section as AdminSection : null
}

function locationKey() {
  return `${window.location.pathname}${window.location.search}`
}

export function navigateTo(path: string, replace = false) {
  if (locationKey() === path) return
  window.history[replace ? 'replaceState' : 'pushState'](null, '', path)
  window.dispatchEvent(new PopStateEvent('popstate'))
}

export function replaceLocation(path: string) {
  window.history.replaceState(null, '', path)
}

export function useAdminSection() {
  const [section, setSection] = useState<AdminSection | null>(() => sectionFromPath(window.location.pathname))
  useEffect(() => {
    const update = () => setSection(sectionFromPath(window.location.pathname))
    window.addEventListener('popstate', update)
    return () => window.removeEventListener('popstate', update)
  }, [])
  return section
}
