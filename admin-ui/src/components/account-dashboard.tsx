import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { LogOut, Moon, ShieldCheck, Sun, UsersRound, Wallet, KeyRound } from 'lucide-react'
import { getAccount, getTransactions } from '@/api/accounts'
import { storage } from '@/lib/storage'
import type { AccountOverview } from '@/types/accounts'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { UsersPanel } from '@/components/users-panel'
import { TransactionsPanel } from '@/components/transactions-panel'
import { SettingsPanel } from '@/components/settings-panel'

export function formatCredits(micro: number) {
  return (micro / 1_000_000).toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 6 })
}

export function roleName(role: string) {
  return role === 'super_admin' ? '超级管理员' : role === 'admin' ? '管理员' : '用户'
}

export function AccountDashboard({ initialAccount, onLogout, onOpenCredentials }: { initialAccount: AccountOverview; onLogout: () => void; onOpenCredentials: () => void }) {
  const isManager = initialAccount.user.role !== 'user'
  const isSuperAdmin = initialAccount.user.role === 'super_admin'
  const [tab, setTab] = useState<'overview' | 'users' | 'transactions' | 'credentials' | 'settings'>(isManager ? 'users' : 'overview')
  const [dark, setDark] = useState(document.documentElement.classList.contains('dark'))
  const queryClient = useQueryClient()
  const { data: account = initialAccount } = useQuery({ queryKey: ['account'], queryFn: getAccount, initialData: initialAccount })
  const { data: transactions = [] } = useQuery({ queryKey: ['transactions', 'mine'], queryFn: () => getTransactions() })

  const logout = () => {
    storage.removeApiKey()
    queryClient.clear()
    onLogout()
  }
  const selectTab = (value: string) => {
    if (value === 'credentials') {
      onOpenCredentials()
      return
    }
    setTab(value as typeof tab)
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <div className="flex items-center gap-2"><h1 className="text-xl font-semibold tracking-tight">账户与积分</h1><Badge variant={isSuperAdmin ? 'default' : 'secondary'}>{roleName(account.user.role)}</Badge></div>
          <p className="text-sm text-muted-foreground mt-1">{account.user.username} · 独立身份、用量和账单</p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="icon" title="切换主题" onClick={() => { setDark(!dark); document.documentElement.classList.toggle('dark') }}>{dark ? <Sun /> : <Moon />}</Button>
          <Button variant="ghost" size="icon" title="退出登录" onClick={logout}><LogOut /></Button>
        </div>
      </div>
      <div className="flex gap-1 overflow-x-auto border-b" role="tablist">
        {isManager && <TabButton active={tab === 'users'} onClick={() => selectTab('users')} icon={<UsersRound />} label="用户管理" />}
        <TabButton active={tab === 'overview'} onClick={() => selectTab('overview')} icon={<Wallet />} label="我的账户" />
        <TabButton active={tab === 'transactions'} onClick={() => selectTab('transactions')} icon={<Wallet />} label="积分流水" />
        {isSuperAdmin && <TabButton active={tab === 'credentials'} onClick={() => selectTab('credentials')} icon={<ShieldCheck />} label="上游凭据" />}
        <TabButton active={tab === 'settings'} onClick={() => selectTab('settings')} icon={<KeyRound />} label="安全与设置" />
      </div>
      <div className="animate-in fade-in-0 duration-200" key={tab}>
        {tab === 'users' && isManager && <UsersPanel currentUser={account.user} />}
        {tab === 'overview' && <Overview account={account} transactionCount={transactions.length} onShowTransactions={() => setTab('transactions')} />}
        {tab === 'transactions' && <TransactionsPanel isManager={isManager} />}
        {tab === 'settings' && <SettingsPanel account={account} onKeyChanged={logout} />}
      </div>
    </div>
  )
}

function TabButton({ active, onClick, icon, label }: { active: boolean; onClick: () => void; icon: React.ReactNode; label: string }) {
  return <button role="tab" aria-selected={active} onClick={onClick} className={`relative flex min-h-11 items-center gap-2 whitespace-nowrap px-3 text-sm font-medium transition-colors ${active ? 'text-foreground after:absolute after:inset-x-2 after:bottom-0 after:h-0.5 after:rounded-full after:bg-primary' : 'text-muted-foreground hover:text-foreground'}`}>{icon}{label}</button>
}

function Overview({ account, transactionCount, onShowTransactions }: { account: AccountOverview; transactionCount: number; onShowTransactions: () => void }) {
  return (
    <div className="space-y-6">
      <div className="grid gap-4 md:grid-cols-3">
        <Card><CardHeader className="pb-2"><CardTitle className="text-sm font-medium text-muted-foreground">可用积分</CardTitle></CardHeader><CardContent><div className="text-3xl font-semibold tabular-nums">{formatCredits(account.user.balanceMicro)}</div><p className="text-xs text-muted-foreground mt-2">{account.pendingCount} 笔请求正在结算</p></CardContent></Card>
        <Card><CardHeader className="pb-2"><CardTitle className="text-sm font-medium text-muted-foreground">累计消费</CardTitle></CardHeader><CardContent><div className="text-3xl font-semibold tabular-nums">{formatCredits(account.totalSpentMicro)}</div><p className="text-xs text-muted-foreground mt-2">按实际 token 用量结算</p></CardContent></Card>
        <Card><CardHeader className="pb-2"><CardTitle className="text-sm font-medium text-muted-foreground">累计 Token</CardTitle></CardHeader><CardContent><div className="text-3xl font-semibold tabular-nums">{(account.totalInputTokens + account.totalOutputTokens).toLocaleString()}</div><p className="text-xs text-muted-foreground mt-2">输入 {account.totalInputTokens.toLocaleString()} · 输出 {account.totalOutputTokens.toLocaleString()}</p></CardContent></Card>
      </div>
      <Card><CardContent className="flex flex-col gap-4 p-5 sm:flex-row sm:items-center sm:justify-between"><div><h2 className="font-medium">当前计费标准</h2><p className="text-sm text-muted-foreground mt-1">每百万输入 token {account.billing.inputPerMillion} 积分 · 每百万输出 token {account.billing.outputPerMillion} 积分</p></div><Button variant="outline" onClick={onShowTransactions}>查看 {transactionCount} 笔流水</Button></CardContent></Card>
    </div>
  )
}
