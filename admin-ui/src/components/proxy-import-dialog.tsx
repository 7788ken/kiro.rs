import { useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import { formControlClass, textareaClass } from '@/components/ui/input'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { useImportProxies } from '@/hooks/use-proxies'
import { proxyStatusMeta } from '@/lib/proxy-status'
import { extractErrorMessage } from '@/lib/utils'
import type { ImportProxiesResponse, ProxyImportLineIssue, ProxyImportScheme } from '@/types/api'

const PLACEHOLDER = [
  '每行一个代理，支持以下格式：',
  'socks5://user:pass@1.2.3.4:1080',
  'http://proxy.example.com:3128',
  '1.2.3.4:1080:user:pass（或 1.2.3.4:1080，使用下方默认协议）',
  '# 以 # 开头的行会被忽略',
].join('\n')

const SCHEMES: ProxyImportScheme[] = ['socks5', 'socks5h', 'http', 'https']

function countLines(text: string) {
  return text.split(/\r?\n/).filter(line => {
    const trimmed = line.trim()
    return trimmed !== '' && !trimmed.startsWith('#')
  }).length
}

export function ProxyImportDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const [text, setText] = useState('')
  const [defaultScheme, setDefaultScheme] = useState<ProxyImportScheme>('socks5')
  const [checkAfterImport, setCheckAfterImport] = useState(true)
  const [result, setResult] = useState<ImportProxiesResponse | null>(null)
  const importProxies = useImportProxies()
  const pending = importProxies.isPending
  const lineCount = useMemo(() => countLines(text), [text])

  useEffect(() => {
    if (!open) return
    setText('')
    setResult(null)
  }, [open])

  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    if (lineCount === 0) return toast.error('请粘贴至少一行代理')
    importProxies.mutate({ text, defaultScheme, checkAfterImport }, {
      onSuccess: res => {
        setResult(res)
        const summary = `新增 ${res.added.length}，跳过 ${res.skipped.length}，失败 ${res.failed.length}`
        if (res.added.length > 0) toast.success(`导入完成：${summary}`)
        else toast.warning(`没有新增代理：${summary}`)
      },
      onError: error => toast.error('导入失败: ' + extractErrorMessage(error)),
    })
  }

  const continueImport = () => { setText(''); setResult(null) }

  return <Dialog open={open} onOpenChange={next => { if (!pending) onOpenChange(next) }}>
    <DialogContent className="sm:max-w-2xl">
      <DialogHeader>
        <DialogTitle>批量导入代理</DialogTitle>
        <DialogDescription>名称自动生成为 host:port。与已有代理地址和用户名相同的行会被跳过。</DialogDescription>
      </DialogHeader>
      {result ? <ImportResult result={result} checked={checkAfterImport} /> : <form id="proxy-import-form" onSubmit={submit} className="space-y-4">
        <div className="space-y-2">
          <label htmlFor="proxy-import-text" className="text-sm font-medium">代理列表</label>
          <textarea id="proxy-import-text" className={textareaClass} value={text} onChange={e => setText(e.target.value)} placeholder={PLACEHOLDER} spellCheck={false} autoComplete="off" disabled={pending} />
          <p className="text-xs text-muted-foreground">已识别 {lineCount} 行（忽略空行和 # 注释）</p>
        </div>
        <div className="grid gap-4 sm:grid-cols-2 sm:items-end">
          <div className="space-y-2">
            <label htmlFor="proxy-import-scheme" className="text-sm font-medium">无协议前缀时默认使用</label>
            <select id="proxy-import-scheme" className={formControlClass} value={defaultScheme} onChange={e => setDefaultScheme(e.target.value as ProxyImportScheme)} disabled={pending}>
              {SCHEMES.map(scheme => <option key={scheme} value={scheme}>{scheme}</option>)}
            </select>
          </div>
          <div className="flex h-10 items-center gap-3">
            <Switch id="proxy-import-check" checked={checkAfterImport} onCheckedChange={setCheckAfterImport} disabled={pending} />
            <label htmlFor="proxy-import-check" className="text-sm">导入后立即检测出口 IP</label>
          </div>
        </div>
      </form>}
      <DialogFooter>
        {result ? <>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>关闭</Button>
          <Button type="button" onClick={continueImport}>继续导入</Button>
        </> : <>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={pending}>取消</Button>
          <Button type="submit" form="proxy-import-form" disabled={pending || lineCount === 0}>{pending ? (checkAfterImport ? '导入并检测中…' : '导入中…') : `导入 ${lineCount} 行`}</Button>
        </>}
      </DialogFooter>
    </DialogContent>
  </Dialog>
}

function ImportResult({ result, checked }: { result: ImportProxiesResponse; checked: boolean }) {
  return <div className="max-h-[55dvh] space-y-4 overflow-y-auto pr-1 text-sm">
    <section className="space-y-2">
      <h3 className="font-medium">新增 {result.added.length}</h3>
      {result.added.length > 0 && <ul className="divide-y rounded-lg border">
        {result.added.map(proxy => {
          const meta = proxyStatusMeta[proxy.health.status] ?? proxyStatusMeta.unknown
          return <li key={proxy.id} className="flex items-center justify-between gap-3 px-3 py-2">
            <div className="min-w-0">
              <p className="truncate font-medium" title={proxy.name}>{proxy.name}</p>
              <p className="truncate font-mono text-[11px] text-muted-foreground" title={proxy.url}>#{proxy.id} · {proxy.url}{proxy.username ? ` · ${proxy.username}` : ''}</p>
              {proxy.health.status === 'unhealthy' && proxy.health.lastError && <p className="truncate text-[11px] text-destructive" title={proxy.health.lastError}>{proxy.health.lastError}</p>}
            </div>
            <div className="flex shrink-0 items-center gap-2">
              {proxy.health.egressIp && <span className="font-mono text-xs">{proxy.health.egressIp}</span>}
              <Badge variant={meta.variant}>{checked ? meta.label : '未检测'}</Badge>
            </div>
          </li>
        })}
      </ul>}
    </section>
    <IssueList title="跳过" issues={result.skipped} />
    <IssueList title="失败" issues={result.failed} destructive />
  </div>
}

function IssueList({ title, issues, destructive = false }: { title: string; issues: ProxyImportLineIssue[]; destructive?: boolean }) {
  return <section className="space-y-2">
    <h3 className="font-medium">{title} {issues.length}</h3>
    {issues.length > 0 && <ul className="divide-y rounded-lg border">
      {issues.map((issue, index) => <li key={`${issue.line}-${index}`} className="flex gap-3 px-3 py-2">
        <span className="shrink-0 tabular-nums text-muted-foreground">第 {issue.line} 行</span>
        <span className={`min-w-0 break-all ${destructive ? 'text-destructive' : ''}`}>{issue.reason}</span>
      </li>)}
    </ul>}
  </section>
}
