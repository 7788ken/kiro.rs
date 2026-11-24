import { useState } from 'react'
import { toast } from 'sonner'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input, formControlClass } from '@/components/ui/input'
import { useAddCredential } from '@/hooks/use-credentials'
import { useProxies } from '@/hooks/use-proxies'
import { extractErrorMessage } from '@/lib/utils'
import { proxyOptionLabel } from '@/components/proxy-binding-dialog'

interface AddCredentialDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

type AuthMethod = 'social' | 'idc' | 'api_key'
type ProxyMode = 'auto' | 'pool' | 'direct' | 'manual'

const proxyModeOptions: { value: ProxyMode; label: string; hint: string }[] = [
  { value: 'auto', label: '自动分配（推荐）', hint: '从健康的代理中挑绑定凭据最少的一个；代理池为空时沿用全局代理' },
  { value: 'pool', label: '指定代理', hint: '固定绑定所选代理池条目' },
  { value: 'direct', label: '直连（direct）', hint: '不使用任何代理，即使配置了全局代理' },
  { value: 'manual', label: '手动填写代理地址', hint: '为该凭据单独填写代理，不进入代理池' },
]

export function AddCredentialDialog({ open, onOpenChange }: AddCredentialDialogProps) {
  const [refreshToken, setRefreshToken] = useState('')
  const [kiroApiKey, setKiroApiKey] = useState('')
  const [authMethod, setAuthMethod] = useState<AuthMethod>('social')
  const [authRegion, setAuthRegion] = useState('')
  const [apiRegion, setApiRegion] = useState('')
  const [clientId, setClientId] = useState('')
  const [clientSecret, setClientSecret] = useState('')
  const [profileArn, setProfileArn] = useState('')
  const [priority, setPriority] = useState('0')
  const [machineId, setMachineId] = useState('')
  const [proxyUrl, setProxyUrl] = useState('')
  const [proxyUsername, setProxyUsername] = useState('')
  const [proxyPassword, setProxyPassword] = useState('')
  const [proxyMode, setProxyMode] = useState<ProxyMode>('auto')
  const [proxyId, setProxyId] = useState('')
  const [endpoint, setEndpoint] = useState('')

  const { mutate, isPending } = useAddCredential()
  const proxiesQuery = useProxies(open)
  const enabledProxies = (proxiesQuery.data?.proxies ?? []).filter(proxy => proxy.enabled)

  const resetForm = () => {
    setRefreshToken('')
    setKiroApiKey('')
    setAuthMethod('social')
    setAuthRegion('')
    setApiRegion('')
    setClientId('')
    setClientSecret('')
    setProfileArn('')
    setPriority('0')
    setMachineId('')
    setProxyUrl('')
    setProxyUsername('')
    setProxyPassword('')
    setProxyMode('auto')
    setProxyId('')
    setEndpoint('')
  }

  const isApiKey = authMethod === 'api_key'

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    // 验证必填字段
    if (isApiKey) {
      if (!kiroApiKey.trim()) {
        toast.error('请输入 Kiro API Key')
        return
      }
    } else {
      if (!refreshToken.trim()) {
        toast.error('请输入 Refresh Token')
        return
      }
      // IdC/Builder-ID/IAM 需要额外字段
      if (authMethod === 'idc' && (!clientId.trim() || !clientSecret.trim())) {
        toast.error('IdC/Builder-ID/IAM 认证需要填写 Client ID 和 Client Secret')
        return
      }
    }

    if (proxyMode === 'pool' && !proxyId) {
      toast.error('请选择要绑定的代理')
      return
    }
    if (proxyMode === 'manual' && !proxyUrl.trim()) {
      toast.error('请填写代理地址')
      return
    }
    const manual = proxyMode === 'manual'

    mutate(
      {
        authMethod,
        profileArn: isApiKey ? undefined : profileArn.trim() || undefined,
        refreshToken: isApiKey ? undefined : refreshToken.trim(),
        kiroApiKey: isApiKey ? kiroApiKey.trim() : undefined,
        authRegion: authRegion.trim() || undefined,
        apiRegion: apiRegion.trim() || undefined,
        clientId: isApiKey ? undefined : clientId.trim() || undefined,
        clientSecret: isApiKey ? undefined : clientSecret.trim() || undefined,
        priority: parseInt(priority) || 0,
        machineId: machineId.trim() || undefined,
        proxyUrl: proxyMode === 'direct' ? 'direct' : manual ? proxyUrl.trim() : undefined,
        proxyUsername: manual ? proxyUsername.trim() || undefined : undefined,
        proxyPassword: manual ? proxyPassword.trim() || undefined : undefined,
        proxyId: proxyMode === 'pool' ? Number(proxyId) : undefined,
        endpoint: endpoint.trim() || undefined,
      },
      {
        onSuccess: (data) => {
          toast.success(data.message)
          onOpenChange(false)
          resetForm()
        },
        onError: (error: unknown) => {
          toast.error(`添加失败: ${extractErrorMessage(error)}`)
        },
      }
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg max-h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>添加凭据</DialogTitle>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="flex flex-col min-h-0 flex-1">
          <div className="space-y-4 py-4 overflow-y-auto flex-1 pr-1">
            {/* 认证方式 */}
            <div className="space-y-2">
              <label htmlFor="authMethod" className="text-sm font-medium">
                认证方式
              </label>
              <select
                id="authMethod"
                value={authMethod}
                onChange={(e) => setAuthMethod(e.target.value as AuthMethod)}
                disabled={isPending}
                className={formControlClass}
              >
                <option value="social">Social</option>
                <option value="idc">IdC/Builder-ID/IAM</option>
                <option value="api_key">API Key</option>
              </select>
            </div>

            {/* Kiro API Key (API Key 模式) */}
            {isApiKey && (
              <div className="space-y-2">
                <label htmlFor="kiroApiKey" className="text-sm font-medium">
                  Kiro API Key <span className="text-red-500">*</span>
                </label>
                <Input
                  id="kiroApiKey"
                  type="password"
                  placeholder="格式: ksk_xxxxxxxx"
                  value={kiroApiKey}
                  onChange={(e) => setKiroApiKey(e.target.value)}
                  disabled={isPending}
                />
              </div>
            )}

            {/* Refresh Token (OAuth 模式) */}
            {!isApiKey && (
              <div className="space-y-2">
                <label htmlFor="refreshToken" className="text-sm font-medium">
                  Refresh Token <span className="text-red-500">*</span>
                </label>
                <Input
                  id="refreshToken"
                  type="password"
                  placeholder="请输入 Refresh Token"
                  value={refreshToken}
                  onChange={(e) => setRefreshToken(e.target.value)}
                  disabled={isPending}
                />
              </div>
            )}

            {/* Region 配置 */}
            <div className="space-y-2">
              <label className="text-sm font-medium">Region 配置</label>
              <div className="grid grid-cols-2 gap-2">
                <div>
                  <Input
                    id="authRegion"
                    placeholder="Auth Region"
                    value={authRegion}
                    onChange={(e) => setAuthRegion(e.target.value)}
                    disabled={isPending}
                  />
                </div>
                <div>
                  <Input
                    id="apiRegion"
                    placeholder="API Region"
                    value={apiRegion}
                    onChange={(e) => setApiRegion(e.target.value)}
                    disabled={isPending}
                  />
                </div>
              </div>
              <p className="text-xs text-muted-foreground">
                均可留空使用全局配置。Auth Region 用于 Token 刷新，API Region 用于 API 请求
              </p>
            </div>

            {/* IdC/Builder-ID/IAM 额外字段 */}
            {authMethod === 'idc' && (
              <>
                <div className="space-y-2">
                  <label htmlFor="profileArn" className="text-sm font-medium">Profile ARN</label>
                  <Input id="profileArn" value={profileArn} onChange={e => setProfileArn(e.target.value)} placeholder="arn:aws:codewhisperer:...（企业 IdC 建议填写）" disabled={isPending} />
                  <p className="text-xs text-muted-foreground">用于企业账号的额度查询和 API 调用；缺少时可能导致额度查询失败。</p>
                </div>
                <div className="space-y-2">
                  <label htmlFor="clientId" className="text-sm font-medium">
                    Client ID <span className="text-red-500">*</span>
                  </label>
                  <Input
                    id="clientId"
                    placeholder="请输入 Client ID"
                    value={clientId}
                    onChange={(e) => setClientId(e.target.value)}
                    disabled={isPending}
                  />
                </div>
                <div className="space-y-2">
                  <label htmlFor="clientSecret" className="text-sm font-medium">
                    Client Secret <span className="text-red-500">*</span>
                  </label>
                  <Input
                    id="clientSecret"
                    type="password"
                    placeholder="请输入 Client Secret"
                    value={clientSecret}
                    onChange={(e) => setClientSecret(e.target.value)}
                    disabled={isPending}
                  />
                </div>
              </>
            )}

            {/* 优先级 */}
            <div className="space-y-2">
              <label htmlFor="priority" className="text-sm font-medium">
                优先级
              </label>
              <Input
                id="priority"
                type="number"
                min="0"
                placeholder="数字越小优先级越高"
                value={priority}
                onChange={(e) => setPriority(e.target.value)}
                disabled={isPending}
              />
              <p className="text-xs text-muted-foreground">
                数字越小优先级越高，默认为 0
              </p>
            </div>

            {/* Machine ID */}
            <div className="space-y-2">
              <label htmlFor="machineId" className="text-sm font-medium">
                Machine ID
              </label>
              <Input
                id="machineId"
                placeholder="留空使用配置中字段, 否则由刷新Token自动派生"
                value={machineId}
                onChange={(e) => setMachineId(e.target.value)}
                disabled={isPending}
              />
              <p className="text-xs text-muted-foreground">
                可选，64 位十六进制字符串，留空使用配置中字段, 否则由刷新Token自动派生
              </p>
            </div>

            {/* 端点 */}
            <div className="space-y-2">
              <label htmlFor="endpoint" className="text-sm font-medium">
                端点
              </label>
              <Input
                id="endpoint"
                placeholder="留空使用默认端点（如 ide / cli）"
                value={endpoint}
                onChange={(e) => setEndpoint(e.target.value)}
                disabled={isPending}
              />
              <p className="text-xs text-muted-foreground">
                可选。决定该凭据走哪套 Kiro API。留空使用全局 defaultEndpoint
              </p>
            </div>

            {/* 代理配置 */}
            <div className="space-y-2">
              <span className="text-sm font-medium">出口代理</span>
              <div role="radiogroup" aria-label="出口代理" className="grid gap-2 sm:grid-cols-2">
                {proxyModeOptions.map(option => (
                  <label key={option.value} className={`flex cursor-pointer items-start gap-2 rounded-lg border px-3 py-2 text-sm transition-colors ${proxyMode === option.value ? 'border-primary/50 bg-primary/[0.04]' : 'hover:bg-muted/40'}`}>
                    <input type="radio" name="proxyMode" value={option.value} checked={proxyMode === option.value} onChange={() => setProxyMode(option.value)} disabled={isPending} className="mt-0.5 accent-[hsl(var(--primary))]" />
                    <span><span className="font-medium">{option.label}</span><span className="mt-0.5 block text-xs text-muted-foreground">{option.hint}</span></span>
                  </label>
                ))}
              </div>
              {proxyMode === 'pool' && (
                <select
                  id="proxyId"
                  aria-label="选择代理"
                  value={proxyId}
                  onChange={(e) => setProxyId(e.target.value)}
                  disabled={isPending || proxiesQuery.isLoading}
                  className={formControlClass}
                >
                  <option value="">{proxiesQuery.isLoading ? '正在加载代理…' : enabledProxies.length ? '请选择代理' : '暂无启用的代理'}</option>
                  {enabledProxies.map(proxy => <option key={proxy.id} value={proxy.id}>{proxyOptionLabel(proxy)}</option>)}
                </select>
              )}
              {proxyMode === 'manual' && (
                <>
                  <Input
                    id="proxyUrl"
                    placeholder="代理 URL，如 http://host:port 或 socks5://host:port"
                    value={proxyUrl}
                    onChange={(e) => setProxyUrl(e.target.value)}
                    disabled={isPending}
                  />
                  <div className="grid grid-cols-2 gap-2">
                    <Input
                      id="proxyUsername"
                      placeholder="代理用户名"
                      value={proxyUsername}
                      onChange={(e) => setProxyUsername(e.target.value)}
                      disabled={isPending}
                    />
                    <Input
                      id="proxyPassword"
                      type="password"
                      placeholder="代理密码"
                      value={proxyPassword}
                      onChange={(e) => setProxyPassword(e.target.value)}
                      disabled={isPending}
                    />
                  </div>
                </>
              )}
            </div>
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={isPending}
            >
              取消
            </Button>
            <Button type="submit" disabled={isPending}>
              {isPending ? '添加中...' : '添加'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
