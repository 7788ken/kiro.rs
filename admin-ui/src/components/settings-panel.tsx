import { useState } from 'react'
import { toast } from 'sonner'
import { KeyRound, Save } from 'lucide-react'
import { changeOwnKey, changePassword, setBilling } from '@/api/accounts'
import { storage } from '@/lib/storage'
import { extractErrorMessage } from '@/lib/utils'
import type { AccountOverview } from '@/types/accounts'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { KeyDialog } from '@/components/key-dialog'

export function SettingsPanel({ account, onKeyChanged }: { account: AccountOverview; onKeyChanged: () => void }) {
  const isSuperAdmin = account.user.role === 'super_admin'
  const [inputRate, setInputRate] = useState(String(account.billing.inputPerMillion))
  const [outputRate, setOutputRate] = useState(String(account.billing.outputPerMillion))
  const [currentPassword, setCurrentPassword] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [newKey, setNewKey] = useState('')
  const [pending, setPending] = useState(false)

  const saveBilling = async (event: React.FormEvent) => {
    event.preventDefault()
    setPending(true)
    try {
      await setBilling({ inputPerMillion: Number(inputRate), outputPerMillion: Number(outputRate) })
      toast.success('计费单价已保存，新请求将按新单价预授权')
    } catch (error) { toast.error(extractErrorMessage(error)) } finally { setPending(false) }
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
    <div className="grid gap-5 lg:grid-cols-2">
      {isSuperAdmin && <Card><CardHeader><CardTitle className="text-base">计费单价</CardTitle><CardDescription>单位：积分 / 百万 token。请求开始时锁定价格，完成时按实际用量结算。</CardDescription></CardHeader><CardContent><form onSubmit={saveBilling} className="space-y-4"><div className="grid grid-cols-2 gap-4"><div className="space-y-2"><label htmlFor="input-rate" className="text-sm font-medium">输入单价</label><Input id="input-rate" type="number" min="0" step="0.000001" value={inputRate} onChange={event => setInputRate(event.target.value)} /></div><div className="space-y-2"><label htmlFor="output-rate" className="text-sm font-medium">输出单价</label><Input id="output-rate" type="number" min="0" step="0.000001" value={outputRate} onChange={event => setOutputRate(event.target.value)} /></div></div><Button disabled={pending}><Save />保存单价</Button></form></CardContent></Card>}
      <Card><CardHeader><CardTitle className="text-base">API Key</CardTitle><CardDescription>完整 Key 只在重置后展示一次，数据库仅保存哈希。</CardDescription></CardHeader><CardContent><Button variant="outline" disabled={pending} onClick={rotateKey}><KeyRound />重置我的 API Key</Button></CardContent></Card>
      <Card><CardHeader><CardTitle className="text-base">修改密码</CardTitle><CardDescription>修改后需要重新登录，API Key 也会轮换。</CardDescription></CardHeader><CardContent><form onSubmit={updatePassword} className="space-y-3"><Input type="password" value={currentPassword} onChange={event => setCurrentPassword(event.target.value)} placeholder="当前密码" autoComplete="current-password" required /><Input type="password" value={newPassword} onChange={event => setNewPassword(event.target.value)} placeholder="新密码（至少 10 个字符）" autoComplete="new-password" minLength={10} required /><Button disabled={pending}>更新密码</Button></form></CardContent></Card>
      <KeyDialog apiKey={newKey} onClose={() => setNewKey('')} title="我的新 API Key" />
    </div>
  )
}
