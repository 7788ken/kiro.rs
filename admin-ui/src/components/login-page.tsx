import { useState } from 'react'
import { toast } from 'sonner'
import { KeyRound, LogIn } from 'lucide-react'
import { loginWithPassword } from '@/api/accounts'
import { storage } from '@/lib/storage'
import { extractErrorMessage } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import type { AccountOverview } from '@/types/accounts'

export function LoginPage({ onLogin }: { onLogin: (account: AccountOverview) => void }) {
  const [key, setKey] = useState('')
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [pending, setPending] = useState(false)

  const submitKey = async (event: React.FormEvent) => {
    event.preventDefault()
    if (!key.trim()) return
    setPending(true)
    storage.setApiKey(key.trim())
    try {
      const { getAccount } = await import('@/api/accounts')
      const account = await getAccount()
      onLogin(account)
    } catch (error) {
      storage.removeApiKey()
      toast.error(`登录失败：${extractErrorMessage(error)}`)
    } finally {
      setPending(false)
    }
  }

  const submitPassword = async (event: React.FormEvent) => {
    event.preventDefault()
    setPending(true)
    try {
      const result = await loginWithPassword(username.trim(), password)
      storage.setApiKey(result.sessionToken)
      onLogin(result.account)
    } catch (error) {
      toast.error(`登录失败：${extractErrorMessage(error)}`)
    } finally {
      setPending(false)
    }
  }

  return (
    <div className="min-h-screen bg-background flex items-center justify-center px-4 py-10 antialiased">
      <div className="w-full max-w-md space-y-6">
        <div className="text-center space-y-2">
          <div className="mx-auto h-11 w-11 rounded-xl bg-primary text-primary-foreground flex items-center justify-center shadow-sm">
            <KeyRound className="h-5 w-5" />
          </div>
          <h1 className="text-2xl font-semibold tracking-tight text-balance">Kiro 账户中心</h1>
          <p className="text-sm text-muted-foreground">管理用户与积分，或查看自己的消费记录</p>
        </div>
        <Card>
          <CardHeader>
            <CardTitle className="text-base">使用 API Key</CardTitle>
            <CardDescription>支持管理员密钥和用户专属密钥</CardDescription>
          </CardHeader>
          <CardContent>
            <form onSubmit={submitKey} className="space-y-3">
              <Input type="password" value={key} onChange={event => setKey(event.target.value)} placeholder="sk-..." autoComplete="off" />
              <Button className="w-full" disabled={!key.trim() || pending}><LogIn />登录</Button>
            </form>
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle className="text-base">使用账户密码</CardTitle>
            <CardDescription>使用独立网页会话，不会影响正在使用的 API Key</CardDescription>
          </CardHeader>
          <CardContent>
            <form onSubmit={submitPassword} className="space-y-3">
              <Input value={username} onChange={event => setUsername(event.target.value)} placeholder="用户名" autoComplete="username" />
              <Input type="password" value={password} onChange={event => setPassword(event.target.value)} placeholder="密码" autoComplete="current-password" />
              <Button variant="outline" className="w-full" disabled={!username.trim() || !password || pending}>密码登录</Button>
            </form>
          </CardContent>
        </Card>
      </div>
    </div>
  )
}
