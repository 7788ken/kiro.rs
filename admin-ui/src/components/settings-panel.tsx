import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { KeyRound, Save } from 'lucide-react'
import { CoreConfigPanel } from '@/components/core-config-panel'
import { changeOwnKey, changePassword, setBilling } from '@/api/accounts'
import { storage } from '@/lib/storage'
import { extractErrorMessage } from '@/lib/utils'
import type { AccountOverview, BillingConfig } from '@/types/accounts'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { KeyDialog } from '@/components/key-dialog'

export function SettingsPanel({ account, onKeyChanged }: { account: AccountOverview; onKeyChanged: () => void }) {
  const isSuperAdmin = account.user.role === 'super_admin'
  const queryClient = useQueryClient()
  const [savedBilling, setSavedBilling] = useState<BillingConfig>(account.billing)
  const [inputRate, setInputRate] = useState(String(account.billing.inputPerMillion))
  const [outputRate, setOutputRate] = useState(String(account.billing.outputPerMillion))
  const [consumptionRatio, setConsumptionRatio] = useState(String(account.billing.consumptionRatio ?? 1))
  const [currentPassword, setCurrentPassword] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [newKey, setNewKey] = useState('')
  const [pending, setPending] = useState(false)
  const [savingPrice, setSavingPrice] = useState(false)
  const [savingHold, setSavingHold] = useState(false)

  const persistBilling = async (next: BillingConfig, message: string) => {
    await setBilling(next)
    setSavedBilling(next)
    queryClient.invalidateQueries({ queryKey: ['account'] })
    toast.success(message)
  }
  const savePrice = async (event: React.FormEvent) => {
    event.preventDefault()
    const ratio = Number(consumptionRatio)
    if (!Number.isFinite(ratio) || ratio <= 0 || ratio > 1000) return toast.error('消费倍率必须大于 0，且不超过 1000')
    setSavingPrice(true)
    try {
      await persistBilling({ ...savedBilling, consumptionRatio: ratio }, '计费单价已保存，新请求按这个倍率结算')
    } catch (error) { toast.error(extractErrorMessage(error)) } finally { setSavingPrice(false) }
  }
  const saveHold = async (event: React.FormEvent) => {
    event.preventDefault()
    const inputPerMillion = Number(inputRate)
    const outputPerMillion = Number(outputRate)
    const valid = (value: number) => Number.isFinite(value) && value >= 0 && value <= 1_000_000_000
    if (!valid(inputPerMillion) || !valid(outputPerMillion)) return toast.error('预扣单价必须在 0 至 1,000,000,000 之间')
    setSavingHold(true)
    try {
      await persistBilling({ ...savedBilling, inputPerMillion, outputPerMillion }, '预扣费已保存，只影响新请求开始时冻结的余额')
    } catch (error) { toast.error(extractErrorMessage(error)) } finally { setSavingHold(false) }
  }
  const rotateKey = async () => {
    if (!confirm('重置后，使用当前 Key 的客户端会立即失效，确定继续吗？')) return
    setPending(true)
    try {
      const result = await changeOwnKey()
      storage.setApiKey(result.apiKey)
      setNewKey(result.apiKey)
    } catch (error) { toast.error(extractErrorMessage(error)) } finally { setPending(false) }
  }
  const updatePassword = async (event: React.FormEvent) => {
    event.preventDefault()
    if (newPassword.length < 10) return toast.error('新密码至少需要 10 个字符')
    setPending(true)
    try {
      await changePassword(currentPassword, newPassword)
      toast.success('密码已修改，请重新登录')
      onKeyChanged()
    } catch (error) { toast.error(extractErrorMessage(error)) } finally { setPending(false) }
  }

  return (
    <div className="space-y-7">
      {isSuperAdmin && <CoreConfigPanel />}
      <div className="grid gap-5 lg:grid-cols-2">
      {isSuperAdmin && <Card><CardHeader><CardTitle className="text-base">计费单价</CardTitle><CardDescription>最终扣费 = 上游本次实际消耗 × 消费倍率。1 是 1:1，1.2 表示上游扣 1、向用户收 1.2。</CardDescription></CardHeader><CardContent><form onSubmit={savePrice} className="space-y-4"><div className="space-y-2"><label htmlFor="consumption-ratio" className="text-sm font-medium">消费倍率</label><Input id="consumption-ratio" type="number" min="0.001" step="0.001" value={consumptionRatio} onChange={event => setConsumptionRatio(event.target.value)} /></div><Button disabled={savingPrice}><Save />保存单价</Button></form></CardContent></Card>}
      {isSuperAdmin && <Card><CardHeader><CardTitle className="text-base">预扣费</CardTitle><CardDescription>请求开始时按 token 估算并冻结余额，结算时再按计费单价多退少补。这里的单价不决定最终扣多少。</CardDescription></CardHeader><CardContent><form onSubmit={saveHold} className="space-y-4"><div className="grid grid-cols-2 gap-4"><div className="space-y-2"><label htmlFor="input-rate" className="text-sm font-medium">输入单价</label><Input id="input-rate" type="number" min="0" step="0.000001" value={inputRate} onChange={event => setInputRate(event.target.value)} /><p className="text-xs text-muted-foreground">积分 / 百万 token</p></div><div className="space-y-2"><label htmlFor="output-rate" className="text-sm font-medium">输出单价</label><Input id="output-rate" type="number" min="0" step="0.000001" value={outputRate} onChange={event => setOutputRate(event.target.value)} /><p className="text-xs text-muted-foreground">积分 / 百万 token</p></div></div><Button disabled={savingHold}><Save />保存预扣</Button></form></CardContent></Card>}
      <Card><CardHeader><CardTitle className="text-base">API Key</CardTitle><CardDescription>完整 Key 只在重置后展示一次，数据库仅保存哈希。</CardDescription></CardHeader><CardContent><Button variant="outline" disabled={pending} onClick={rotateKey}><KeyRound />重置我的 API Key</Button></CardContent></Card>
      <Card><CardHeader><CardTitle className="text-base">修改密码</CardTitle><CardDescription>修改后需要重新登录，API Key 也会轮换。</CardDescription></CardHeader><CardContent><form onSubmit={updatePassword} className="space-y-3"><Input type="password" value={currentPassword} onChange={event => setCurrentPassword(event.target.value)} placeholder="当前密码" autoComplete="current-password" required /><Input type="password" value={newPassword} onChange={event => setNewPassword(event.target.value)} placeholder="新密码（至少 10 个字符）" autoComplete="new-password" minLength={10} required /><Button disabled={pending}>更新密码</Button></form></CardContent></Card>
      <KeyDialog apiKey={newKey} onClose={() => setNewKey('')} title="我的新 API Key" />
      </div>
    </div>
  )
}
