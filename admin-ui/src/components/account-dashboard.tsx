import { useQuery } from '@tanstack/react-query'
import { Coins, TrendingUp, Wallet } from 'lucide-react'
import { getAccount, getTransactions } from '@/api/accounts'
import type { AccountOverview } from '@/types/accounts'
import { Card, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { UsersPanel } from '@/components/users-panel'
import { TransactionsPanel } from '@/components/transactions-panel'
import { SettingsPanel } from '@/components/settings-panel'
import { MetricCard, PageSection } from '@/components/page-elements'
import type { AdminSection } from '@/components/admin-shell'

export function formatCredits(micro: number) {
  return (Math.abs(micro) / 1_000_000).toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 6 })
}

export function AccountDashboard({ initialAccount, section, onSectionChange, onLogout }: { initialAccount: AccountOverview; section: AdminSection; onSectionChange: (section: AdminSection) => void; onLogout: () => void }) {
  const { data: account = initialAccount } = useQuery({ queryKey: ['account'], queryFn: getAccount, initialData: initialAccount })
  const isManager = account.user.role !== 'user'
  const { data: transactions = [] } = useQuery({ queryKey: ['transactions', 'mine'], queryFn: () => getTransactions() })

  return (
    <div className="animate-in fade-in-0 duration-200" key={section}>
      {section === 'users' && isManager && <UsersPanel currentUser={account.user} />}
      {section === 'overview' && <Overview account={account} transactionCount={transactions.length} onShowTransactions={() => onSectionChange('transactions')} />}
      {section === 'transactions' && <TransactionsPanel isManager={isManager} />}
      {section === 'settings' && <SettingsPanel account={account} onKeyChanged={onLogout} />}
      {section === 'credentials' && <div className="rounded-xl border bg-card p-5 text-sm text-muted-foreground">上游凭据仅超级管理员可管理。</div>}
    </div>
  )
}

function Overview({ account, transactionCount, onShowTransactions }: { account: AccountOverview; transactionCount: number; onShowTransactions: () => void }) {
  return (
    <div className="space-y-7">
      <div className="grid gap-4 md:grid-cols-3">
        <MetricCard label="可用积分" value={formatCredits(account.user.balanceMicro)} detail={`${account.pendingCount} 笔请求正在结算`} icon={Wallet} tone="accent" />
        <MetricCard label="累计消费" value={formatCredits(account.totalSpentMicro)} detail="按实际 Token 用量结算" icon={Coins} />
        <MetricCard label="累计 Token" value={(account.totalInputTokens + account.totalOutputTokens).toLocaleString()} detail={`输入 ${account.totalInputTokens.toLocaleString()} · 输出 ${account.totalOutputTokens.toLocaleString()}`} icon={TrendingUp} />
      </div>
      <PageSection title="计费标准" description="请求开始时锁定价格，完成后按实际用量结算。" actions={<Button variant="outline" size="sm" onClick={onShowTransactions}>查看 {transactionCount} 条调用日志</Button>}>
        <Card><CardContent className="grid gap-5 p-5 sm:grid-cols-2"><div><p className="text-sm text-muted-foreground">每百万输入 Token</p><p className="mt-2 text-xl font-semibold tabular-nums">{account.billing.inputPerMillion}<span className="ml-2 text-sm font-normal text-muted-foreground">积分</span></p></div><div><p className="text-sm text-muted-foreground">每百万输出 Token</p><p className="mt-2 text-xl font-semibold tabular-nums">{account.billing.outputPerMillion}<span className="ml-2 text-sm font-normal text-muted-foreground">积分</span></p></div></CardContent></Card>
      </PageSection>
    </div>
  )
}
