import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { ReceiptText } from 'lucide-react'
import { getTransactions, getUsers } from '@/api/accounts'
import type { CreditTransaction } from '@/types/accounts'
import { Card, CardContent } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { formatCredits } from '@/components/account-dashboard'

const kindNames: Record<string, string> = { initial: '初始积分', adjustment: '管理员调整', charge: '调用消费', refund: '余额返还', release: '失败退回', reserve: '预授权' }

export function TransactionsPanel({ isManager }: { isManager: boolean }) {
  const [userId, setUserId] = useState<number | undefined>()
  const { data: users = [] } = useQuery({ queryKey: ['users'], queryFn: getUsers, enabled: isManager })
  const { data: transactions = [], isLoading } = useQuery({ queryKey: ['transactions', userId ?? 'all'], queryFn: () => getTransactions(userId) })
  return (
    <div className="space-y-4">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between"><div><h2 className="text-lg font-semibold">积分流水</h2><p className="text-sm text-muted-foreground">预授权、实际消费与返还均可追溯</p></div>{isManager && <select aria-label="筛选用户" className="h-10 rounded-md border border-input bg-background px-3 text-sm" value={userId ?? ''} onChange={event => setUserId(event.target.value ? Number(event.target.value) : undefined)}><option value="">全部用户</option>{users.map(user => <option value={user.id} key={user.id}>{user.username}</option>)}</select>}</div>
      <Card><CardContent className="p-0"><div className="overflow-x-auto"><table className="w-full text-sm"><thead className="bg-muted/40 text-left text-xs text-muted-foreground"><tr><th className="px-5 py-3 font-medium">时间 / 类型</th>{isManager && <th className="px-4 py-3 font-medium">用户</th>}<th className="px-4 py-3 font-medium">明细</th><th className="px-4 py-3 font-medium text-right">积分变动</th><th className="px-5 py-3 font-medium text-right">变动后余额</th></tr></thead><tbody className="divide-y">
        {transactions.map(transaction => <TransactionRow key={transaction.id} transaction={transaction} isManager={isManager} />)}
        {!isLoading && transactions.length === 0 && <tr><td colSpan={isManager ? 5 : 4} className="px-5 py-16 text-center"><ReceiptText className="h-8 w-8 mx-auto text-muted-foreground mb-3" /><p className="text-muted-foreground">暂无积分流水</p></td></tr>}
      </tbody></table></div></CardContent></Card>
    </div>
  )
}

function TransactionRow({ transaction, isManager }: { transaction: CreditTransaction; isManager: boolean }) {
  const positive = transaction.amountMicro > 0
  return <tr className="hover:bg-muted/20 transition-colors"><td className="px-5 py-4"><div className="flex items-center gap-2"><Badge variant={transaction.kind === 'charge' ? 'default' : transaction.kind === 'adjustment' || transaction.kind === 'initial' ? 'success' : 'secondary'}>{kindNames[transaction.kind] || transaction.kind}</Badge>{transaction.status === 'pending' && <Badge variant="warning">结算中</Badge>}</div><div className="text-xs text-muted-foreground mt-1.5">{new Date(transaction.createdAt).toLocaleString('zh-CN')}</div></td>{isManager && <td className="px-4 py-4">{transaction.username}</td>}<td className="px-4 py-4"><div className="max-w-xs"><div className="truncate">{transaction.model || transaction.note || '—'}</div>{transaction.kind === 'charge' && <div className="text-xs text-muted-foreground mt-1 tabular-nums">输入 {transaction.inputTokens.toLocaleString()} · 输出 {transaction.outputTokens.toLocaleString()}</div>}{transaction.note && transaction.model && <div className="text-xs text-muted-foreground mt-1">{transaction.note}</div>}</div></td><td className={`px-4 py-4 text-right font-semibold tabular-nums ${positive ? 'text-green-600' : ''}`}>{positive ? '+' : ''}{formatCredits(transaction.amountMicro)}</td><td className="px-5 py-4 text-right tabular-nums text-muted-foreground">{formatCredits(transaction.balanceAfterMicro)}</td></tr>
}
