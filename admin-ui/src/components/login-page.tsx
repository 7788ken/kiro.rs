import { useState } from 'react'
import { toast } from 'sonner'
import { KeyRound, Loader2, LogIn, ShieldCheck } from 'lucide-react'
import { getAccount, loginWithPassword } from '@/api/accounts'
import { storage } from '@/lib/storage'
import { extractErrorMessage } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import type { AccountOverview } from '@/types/accounts'

export function LoginPage({ onLogin }: { onLogin: (account: AccountOverview) => void }) {
  const [method, setMethod] = useState<'password' | 'key'>('password')
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
      onLogin(await getAccount())
    } catch (error) {
      storage.removeApiKey()
      toast.error(`登录失败：${extractErrorMessage(error)}`)
    } finally { setPending(false) }
  }

  const submitPassword = async (event: React.FormEvent) => {
    event.preventDefault()
    setPending(true)
    try {
      const result = await loginWithPassword(username.trim(), password)
      storage.setApiKey(result.sessionToken)
      onLogin(result.account)
    } catch (error) { toast.error(`登录失败：${extractErrorMessage(error)}`) } finally { setPending(false) }
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-background px-4 py-10 antialiased">
      <div className="w-full max-w-[400px]">
        <div className="mb-8 text-center"><div className="mx-auto flex h-12 w-12 items-center justify-center rounded-2xl bg-primary text-primary-foreground shadow-[0_8px_24px_-8px_hsl(var(--primary)/0.5)]"><KeyRound className="h-[22px] w-[22px]" /></div><h1 className="mt-5 text-balance text-[26px] font-semibold tracking-[-0.03em]">Kiro Console</h1><p className="mt-2 text-sm text-muted-foreground">统一管理服务、用户与用量</p></div>
        <Card className="shadow-[0_12px_40px_-18px_rgba(15,23,42,0.16)]">
          <CardHeader className="p-6 pb-4"><CardTitle className="text-xl">欢迎回来</CardTitle><CardDescription>登录以进入你的管理工作台</CardDescription></CardHeader>
          <CardContent className="p-6 pt-2">
            <div className="mb-6 grid grid-cols-2 rounded-lg bg-muted p-1" role="tablist" aria-label="登录方式"><button role="tab" aria-selected={method === 'password'} className={`min-h-10 rounded-md text-sm font-medium transition-colors ${method === 'password' ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`} onClick={() => setMethod('password')}>账户密码</button><button role="tab" aria-selected={method === 'key'} className={`min-h-10 rounded-md text-sm font-medium transition-colors ${method === 'key' ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`} onClick={() => setMethod('key')}>API Key</button></div>
            {method === 'password' ? <form onSubmit={submitPassword} className="space-y-4"><div className="space-y-2"><label htmlFor="login-username" className="text-sm font-medium">用户名</label><Input id="login-username" value={username} onChange={event => setUsername(event.target.value)} placeholder="请输入用户名" autoComplete="username" required /></div><div className="space-y-2"><label htmlFor="login-password" className="text-sm font-medium">密码</label><Input id="login-password" type="password" value={password} onChange={event => setPassword(event.target.value)} placeholder="请输入密码" autoComplete="current-password" required /></div><Button className="w-full" disabled={!username.trim() || !password || pending}>{pending ? <Loader2 className="animate-spin" /> : <LogIn />}登录</Button></form> : <form onSubmit={submitKey} className="space-y-4"><div className="space-y-2"><label htmlFor="login-key" className="text-sm font-medium">API Key</label><Input id="login-key" type="password" value={key} onChange={event => setKey(event.target.value)} placeholder="sk-..." autoComplete="off" required /><p className="text-xs leading-5 text-muted-foreground">支持管理员密钥和用户专属密钥</p></div><Button className="w-full" disabled={!key.trim() || pending}>{pending ? <Loader2 className="animate-spin" /> : <LogIn />}使用 Key 登录</Button></form>}
          </CardContent>
        </Card>
        <p className="mt-6 flex items-center justify-center gap-1.5 text-xs text-muted-foreground"><ShieldCheck className="h-3.5 w-3.5" />请通过 HTTPS 访问，并妥善保管登录凭据</p>
      </div>
    </div>
  )
}
