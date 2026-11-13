import { useState, useEffect, useRef } from 'react'
import { RefreshCw, Server, Plus, Upload, FileUp, Trash2, RotateCcw, CheckCircle2, Search, LayoutGrid, List } from 'lucide-react'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { Card, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { CredentialCard } from '@/components/credential-card'
import { BalanceDialog } from '@/components/balance-dialog'
import { AddCredentialDialog } from '@/components/add-credential-dialog'
import { BatchImportDialog } from '@/components/batch-import-dialog'
import { KamImportDialog } from '@/components/kam-import-dialog'
import { BatchVerifyDialog, type VerifyResult } from '@/components/batch-verify-dialog'
import { useCredentials, useDeleteCredential, useResetFailure, useLoadBalancingMode, useSetLoadBalancingMode } from '@/hooks/use-credentials'
import { getCredentialBalance, forceRefreshToken } from '@/api/credentials'
import { extractErrorMessage } from '@/lib/utils'
import type { BalanceResponse } from '@/types/api'
import { CredentialList } from '@/components/credential-list'
import { EmptyState, PageSection } from '@/components/page-elements'
import { Input, formControlClass } from '@/components/ui/input'

interface DashboardProps {
  onLogout: () => void
  isSuperAdmin: boolean
}

export function Dashboard({ onLogout, isSuperAdmin }: DashboardProps) {
  const [selectedCredentialId, setSelectedCredentialId] = useState<number | null>(null)
  const [balanceDialogOpen, setBalanceDialogOpen] = useState(false)
  const [addDialogOpen, setAddDialogOpen] = useState(false)
  const [batchImportDialogOpen, setBatchImportDialogOpen] = useState(false)
  const [kamImportDialogOpen, setKamImportDialogOpen] = useState(false)
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set())
  const [verifyDialogOpen, setVerifyDialogOpen] = useState(false)
  const [verifying, setVerifying] = useState(false)
  const [verifyProgress, setVerifyProgress] = useState({ current: 0, total: 0 })
  const [verifyResults, setVerifyResults] = useState<Map<number, VerifyResult>>(new Map())
  const [balanceMap, setBalanceMap] = useState<Map<number, BalanceResponse>>(new Map())
  const [loadingBalanceIds, setLoadingBalanceIds] = useState<Set<number>>(new Set())
  const [queryingInfo, setQueryingInfo] = useState(false)
  const [queryInfoProgress, setQueryInfoProgress] = useState({ current: 0, total: 0 })
  const [batchRefreshing, setBatchRefreshing] = useState(false)
  const [batchRefreshProgress, setBatchRefreshProgress] = useState({ current: 0, total: 0 })
  const cancelVerifyRef = useRef(false)
  const [currentPage, setCurrentPage] = useState(1)
  const [viewMode, setViewMode] = useState<'card' | 'list'>(() => localStorage.getItem('kiro-credential-view') === 'list' ? 'list' : 'card')
  const [search, setSearch] = useState('')
  const [statusFilter, setStatusFilter] = useState<'all' | 'enabled' | 'disabled'>('all')
  const [ownerFilter, setOwnerFilter] = useState<'all' | 'manageable' | 'readonly'>('all')
  const itemsPerPage = 12
  const queryClient = useQueryClient()
  const { data, isLoading, error, refetch } = useCredentials()
  const { mutate: deleteCredential } = useDeleteCredential()
  const { mutate: resetFailure } = useResetFailure()
  const { data: loadBalancingData, isLoading: isLoadingMode } = useLoadBalancingMode()
  const { mutate: setLoadBalancingMode, isPending: isSettingMode } = useSetLoadBalancingMode()

  const filteredCredentials = (data?.credentials || []).filter(credential => {
    const keyword = search.trim().toLocaleLowerCase()
    const matchesKeyword = !keyword || [credential.id.toString(), credential.email, credential.endpoint, credential.registeredByName, credential.maskedApiKey, credential.authMethod]
      .some(value => value?.toLocaleLowerCase().includes(keyword))
    const matchesStatus = statusFilter === 'all' || (statusFilter === 'disabled' ? credential.disabled : !credential.disabled)
    const matchesOwner = ownerFilter === 'all' || (ownerFilter === 'manageable' ? credential.canManage : !credential.canManage)
    return matchesKeyword && matchesStatus && matchesOwner
  })
  const totalPages = Math.ceil(filteredCredentials.length / itemsPerPage)
  const startIndex = (currentPage - 1) * itemsPerPage
  const currentCredentials = filteredCredentials.slice(startIndex, startIndex + itemsPerPage)
  const disabledCredentialCount = data?.credentials.filter(credential => credential.canManage && credential.disabled).length || 0
  const manageableCount = data?.credentials.filter(credential => credential.canManage).length || 0
  const selectedDisabledCount = Array.from(selectedIds).filter(id => {
    const credential = data?.credentials.find(c => c.id === id)
    return Boolean(credential?.disabled)
  }).length

  // 当凭据列表变化时重置到第一页
  useEffect(() => {
    setCurrentPage(1)
  }, [data?.credentials.length, search, statusFilter, ownerFilter])

  // 只保留当前仍存在的凭据缓存，避免删除后残留旧数据
  useEffect(() => {
    if (!data?.credentials) {
      setBalanceMap(new Map())
      setLoadingBalanceIds(new Set())
      return
    }

    const validIds = new Set(data.credentials.map(credential => credential.id))

    setBalanceMap(prev => {
      const next = new Map<number, BalanceResponse>()
      prev.forEach((value, id) => {
        if (validIds.has(id)) {
          next.set(id, value)
        }
      })
      return next.size === prev.size ? prev : next
    })

    setLoadingBalanceIds(prev => {
      const next = new Set<number>()
      prev.forEach(id => {
        if (validIds.has(id)) next.add(id)
      })
      return next
    })
    setSelectedIds(previous => new Set(Array.from(previous).filter(id => data.credentials.find(credential => credential.id === id)?.canManage)))
  }, [data?.credentials])

  const handleViewBalance = (id: number) => {
    setSelectedCredentialId(id)
    setBalanceDialogOpen(true)
  }

  const handleRefresh = () => {
    refetch()
    toast.success('已刷新凭据列表')
  }

  const handleLogout = () => {
    queryClient.clear()
    onLogout()
  }

  // 选择管理
  const toggleSelect = (id: number) => {
    if (!data?.credentials.find(credential => credential.id === id)?.canManage) return
    const newSelected = new Set(selectedIds)
    if (newSelected.has(id)) {
      newSelected.delete(id)
    } else {
      newSelected.add(id)
    }
    setSelectedIds(newSelected)
  }

  const deselectAll = () => {
    setSelectedIds(new Set())
  }

  // 批量删除（仅删除已禁用项）
  const handleBatchDelete = async () => {
    if (selectedIds.size === 0) {
      toast.error('请先选择要删除的凭据')
      return
    }

    const disabledIds = Array.from(selectedIds).filter(id => {
      const credential = data?.credentials.find(c => c.id === id)
      return Boolean(credential?.canManage && credential.disabled)
    })

    if (disabledIds.length === 0) {
      toast.error('选中的凭据中没有已禁用项')
      return
    }

    const skippedCount = selectedIds.size - disabledIds.length
    const skippedText = skippedCount > 0 ? `（将跳过 ${skippedCount} 个未禁用凭据）` : ''

    if (!confirm(`确定要删除 ${disabledIds.length} 个已禁用凭据吗？此操作无法撤销。${skippedText}`)) {
      return
    }

    let successCount = 0
    let failCount = 0

    for (const id of disabledIds) {
      try {
        await new Promise<void>((resolve, reject) => {
          deleteCredential(id, {
            onSuccess: () => {
              successCount++
              resolve()
            },
            onError: (err) => {
              failCount++
              reject(err)
            }
          })
        })
      } catch (error) {
        // 错误已在 onError 中处理
      }
    }

    const skippedResultText = skippedCount > 0 ? `，已跳过 ${skippedCount} 个未禁用凭据` : ''

    if (failCount === 0) {
      toast.success(`成功删除 ${successCount} 个已禁用凭据${skippedResultText}`)
    } else {
      toast.warning(`删除已禁用凭据：成功 ${successCount} 个，失败 ${failCount} 个${skippedResultText}`)
    }

    deselectAll()
  }

  // 批量恢复异常
  const handleBatchResetFailure = async () => {
    if (selectedIds.size === 0) {
      toast.error('请先选择要恢复的凭据')
      return
    }

    const failedIds = Array.from(selectedIds).filter(id => {
      const cred = data?.credentials.find(c => c.id === id)
      return cred && cred.canManage && cred.failureCount > 0
    })

    if (failedIds.length === 0) {
      toast.error('选中的凭据中没有失败的凭据')
      return
    }

    let successCount = 0
    let failCount = 0

    for (const id of failedIds) {
      try {
        await new Promise<void>((resolve, reject) => {
          resetFailure(id, {
            onSuccess: () => {
              successCount++
              resolve()
            },
            onError: (err) => {
              failCount++
              reject(err)
            }
          })
        })
      } catch (error) {
        // 错误已在 onError 中处理
      }
    }

    if (failCount === 0) {
      toast.success(`成功恢复 ${successCount} 个凭据`)
    } else {
      toast.warning(`成功 ${successCount} 个，失败 ${failCount} 个`)
    }

    deselectAll()
  }

  // 批量刷新 Token
  const handleBatchForceRefresh = async () => {
    if (selectedIds.size === 0) {
      toast.error('请先选择要刷新的凭据')
      return
    }

    const enabledIds = Array.from(selectedIds).filter(id => {
      const cred = data?.credentials.find(c => c.id === id)
      return cred && cred.canManage && !cred.disabled
    })

    if (enabledIds.length === 0) {
      toast.error('选中的凭据中没有启用的凭据')
      return
    }

    setBatchRefreshing(true)
    setBatchRefreshProgress({ current: 0, total: enabledIds.length })

    let successCount = 0
    let failCount = 0

    for (let i = 0; i < enabledIds.length; i++) {
      try {
        await forceRefreshToken(enabledIds[i])
        successCount++
      } catch {
        failCount++
      }
      setBatchRefreshProgress({ current: i + 1, total: enabledIds.length })
    }

    setBatchRefreshing(false)
    queryClient.invalidateQueries({ queryKey: ['credentials'] })

    if (failCount === 0) {
      toast.success(`成功刷新 ${successCount} 个凭据的 Token`)
    } else {
      toast.warning(`刷新 Token：成功 ${successCount} 个，失败 ${failCount} 个`)
    }

    deselectAll()
  }

  // 一键清除所有已禁用凭据
  const handleClearAll = async () => {
    if (!data?.credentials || data.credentials.length === 0) {
      toast.error('没有可清除的凭据')
      return
    }

    const disabledCredentials = data.credentials.filter(credential => credential.canManage && credential.disabled)

    if (disabledCredentials.length === 0) {
      toast.error('没有可清除的已禁用凭据')
      return
    }

    if (!confirm(`确定要清除所有 ${disabledCredentials.length} 个已禁用凭据吗？此操作无法撤销。`)) {
      return
    }

    let successCount = 0
    let failCount = 0

    for (const credential of disabledCredentials) {
      try {
        await new Promise<void>((resolve, reject) => {
          deleteCredential(credential.id, {
            onSuccess: () => {
              successCount++
              resolve()
            },
            onError: (err) => {
              failCount++
              reject(err)
            }
          })
        })
      } catch (error) {
        // 错误已在 onError 中处理
      }
    }

    if (failCount === 0) {
      toast.success(`成功清除所有 ${successCount} 个已禁用凭据`)
    } else {
      toast.warning(`清除已禁用凭据：成功 ${successCount} 个，失败 ${failCount} 个`)
    }

    deselectAll()
  }

  // 查询当前页凭据信息（逐个查询，避免瞬时并发）
  const handleQueryCurrentPageInfo = async () => {
    if (currentCredentials.length === 0) {
      toast.error('当前页没有可查询的凭据')
      return
    }

    const ids = currentCredentials
      .filter(credential => credential.canManage && !credential.disabled)
      .map(credential => credential.id)

    if (ids.length === 0) {
      toast.error('当前页没有可查询的启用凭据')
      return
    }

    setQueryingInfo(true)
    setQueryInfoProgress({ current: 0, total: ids.length })

    let successCount = 0
    let failCount = 0

    for (let i = 0; i < ids.length; i++) {
      const id = ids[i]

      setLoadingBalanceIds(prev => {
        const next = new Set(prev)
        next.add(id)
        return next
      })

      try {
        const balance = await getCredentialBalance(id)
        successCount++

        setBalanceMap(prev => {
          const next = new Map(prev)
          next.set(id, balance)
          return next
        })
      } catch (error) {
        failCount++
      } finally {
        setLoadingBalanceIds(prev => {
          const next = new Set(prev)
          next.delete(id)
          return next
        })
      }

      setQueryInfoProgress({ current: i + 1, total: ids.length })
    }

    setQueryingInfo(false)

    if (failCount === 0) {
      toast.success(`查询完成：成功 ${successCount}/${ids.length}`)
    } else {
      toast.warning(`查询完成：成功 ${successCount} 个，失败 ${failCount} 个`)
    }
  }

  // 批量验活
  const handleBatchVerify = async () => {
    if (selectedIds.size === 0) {
      toast.error('请先选择要验活的凭据')
      return
    }

    // 初始化状态
    setVerifying(true)
    cancelVerifyRef.current = false
    const ids = Array.from(selectedIds)
    setVerifyProgress({ current: 0, total: ids.length })

    let successCount = 0

    // 初始化结果，所有凭据状态为 pending
    const initialResults = new Map<number, VerifyResult>()
    ids.forEach(id => {
      initialResults.set(id, { id, status: 'pending' })
    })
    setVerifyResults(initialResults)
    setVerifyDialogOpen(true)

    // 开始验活
    for (let i = 0; i < ids.length; i++) {
      // 检查是否取消
      if (cancelVerifyRef.current) {
        toast.info('已取消验活')
        break
      }

      const id = ids[i]

      // 更新当前凭据状态为 verifying
      setVerifyResults(prev => {
        const newResults = new Map(prev)
        newResults.set(id, { id, status: 'verifying' })
        return newResults
      })

      try {
        const balance = await getCredentialBalance(id)
        successCount++

        // 更新为成功状态
        setVerifyResults(prev => {
          const newResults = new Map(prev)
          newResults.set(id, {
            id,
            status: 'success',
            usage: `${balance.currentUsage}/${balance.usageLimit}`
          })
          return newResults
        })
      } catch (error) {
        // 更新为失败状态
        setVerifyResults(prev => {
          const newResults = new Map(prev)
          newResults.set(id, {
            id,
            status: 'failed',
            error: extractErrorMessage(error)
          })
          return newResults
        })
      }

      // 更新进度
      setVerifyProgress({ current: i + 1, total: ids.length })

      // 添加延迟防止封号（最后一个不需要延迟）
      if (i < ids.length - 1 && !cancelVerifyRef.current) {
        await new Promise(resolve => setTimeout(resolve, 2000))
      }
    }

    setVerifying(false)

    if (!cancelVerifyRef.current) {
      toast.success(`验活完成：成功 ${successCount}/${ids.length}`)
    }
  }

  // 取消验活
  const handleCancelVerify = () => {
    cancelVerifyRef.current = true
    setVerifying(false)
  }

  // 切换负载均衡模式
  const handleToggleLoadBalancing = () => {
    const currentMode = loadBalancingData?.mode || 'priority'
    const newMode = currentMode === 'priority' ? 'balanced' : 'priority'

    setLoadBalancingMode(newMode, {
      onSuccess: () => {
        const modeName = newMode === 'priority' ? '优先级模式' : '均衡负载模式'
        toast.success(`已切换到${modeName}`)
      },
      onError: (error) => {
        toast.error(`切换失败: ${extractErrorMessage(error)}`)
      }
    })
  }

  if (isLoading) {
    return (
      <div className="flex min-h-[50vh] items-center justify-center">
        <div className="text-center">
          <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-primary mx-auto mb-4"></div>
          <p className="text-muted-foreground">加载中...</p>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="flex min-h-[50vh] items-center justify-center">
        <Card className="w-full max-w-md">
          <CardContent className="pt-6 text-center">
            <div className="text-red-500 mb-4">加载失败</div>
            <p className="text-muted-foreground mb-4">{(error as Error).message}</p>
            <div className="space-x-2">
              <Button onClick={() => refetch()}>重试</Button>
              <Button variant="outline" onClick={handleLogout}>重新登录</Button>
            </div>
          </CardContent>
        </Card>
      </div>
    )
  }

  const loadBalancingMode = loadBalancingData?.mode || 'priority'

  return (
    <div className="space-y-7">
      <PageSection title="凭据列表" description="可查看全部凭证；只有本人及下级代理登记的凭证可以管理。" actions={<><Button variant="outline" size="sm" onClick={handleToggleLoadBalancing} disabled={!isSuperAdmin || isLoadingMode || isSettingMode} title={!isSuperAdmin ? '仅超级管理员可修改负载均衡模式' : undefined}><RotateCcw />{loadBalancingMode === 'priority' ? '优先级模式' : '均衡负载'}</Button><Button variant="ghost" size="icon" title="刷新凭据列表" aria-label="刷新凭据列表" onClick={handleRefresh}><RefreshCw /></Button></>}>
        <div className="flex flex-col gap-3 rounded-xl bg-muted/35 p-3 lg:flex-row lg:items-center">
          <div className="relative min-w-0 flex-1"><Search className="pointer-events-none absolute left-3 top-3 h-4 w-4 text-muted-foreground" /><Input aria-label="搜索凭据" value={search} onChange={event => setSearch(event.target.value)} placeholder="搜索邮箱、ID、端点或登记人" className="pl-9" /></div>
          <div className="grid grid-cols-2 gap-2 sm:flex">
            <select aria-label="按状态筛选" className={formControlClass + ' sm:w-32'} value={statusFilter} onChange={event => setStatusFilter(event.target.value as typeof statusFilter)}><option value="all">全部状态</option><option value="enabled">已启用</option><option value="disabled">已禁用</option></select>
            <select aria-label="按管理权限筛选" className={formControlClass + ' sm:w-40'} value={ownerFilter} onChange={event => setOwnerFilter(event.target.value as typeof ownerFilter)}><option value="all">全部归属</option><option value="manageable">我可管理</option><option value="readonly">仅可查看</option></select>
          </div>
          {(search || statusFilter !== 'all' || ownerFilter !== 'all') && <Button variant="ghost" size="sm" onClick={() => { setSearch(''); setStatusFilter('all'); setOwnerFilter('all') }}>清除筛选</Button>}
        </div>
        <p className="text-xs tabular-nums text-muted-foreground">找到 {filteredCredentials.length} 个凭证 · 当前分支可管理 {manageableCount} 个</p>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-sm font-medium">卡片 / 列表</p>
          <div className="flex rounded-lg bg-muted p-1" role="group" aria-label="凭据视图"><Button variant={viewMode === 'card' ? 'outline' : 'ghost'} size="sm" aria-pressed={viewMode === 'card'} onClick={() => { setViewMode('card'); localStorage.setItem('kiro-credential-view', 'card') }}><LayoutGrid />卡片</Button><Button variant={viewMode === 'list' ? 'outline' : 'ghost'} size="sm" aria-pressed={viewMode === 'list'} onClick={() => { setViewMode('list'); localStorage.setItem('kiro-credential-view', 'list') }}><List />列表</Button></div>
        </div>
        <div className="flex flex-col gap-3 xl:flex-row xl:items-center xl:justify-between">
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="outline" size="sm" onClick={() => setKamImportDialogOpen(true)}><FileUp />KAM 导入</Button>
            <Button variant="outline" size="sm" onClick={() => setBatchImportDialogOpen(true)}><Upload />批量导入</Button>
            <Button size="sm" onClick={() => setAddDialogOpen(true)}><Plus />添加凭据</Button>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            {data?.credentials && data.credentials.length > 0 && <Button onClick={handleQueryCurrentPageInfo} size="sm" variant="outline" disabled={queryingInfo}><RefreshCw className={queryingInfo ? 'animate-spin' : ''} />{queryingInfo ? `查询 ${queryInfoProgress.current}/${queryInfoProgress.total}` : '查询本页信息'}</Button>}
            {data?.credentials && data.credentials.length > 0 && <Button onClick={handleClearAll} size="sm" variant="ghost" className="text-destructive hover:text-destructive" disabled={disabledCredentialCount === 0} title={disabledCredentialCount === 0 ? '没有可清除的已禁用凭据' : undefined}><Trash2 />清除已禁用</Button>}
          </div>
        </div>

        {selectedIds.size > 0 && <div className="flex flex-col gap-3 rounded-xl border border-primary/15 bg-primary/[0.04] p-3 sm:flex-row sm:items-center sm:justify-between"><div className="flex items-center gap-3 px-1"><Badge>已选择 {selectedIds.size} 个</Badge><button className="min-h-10 text-xs text-muted-foreground hover:text-foreground" onClick={deselectAll}>取消选择</button></div><div className="flex flex-wrap gap-2"><Button onClick={handleBatchVerify} size="sm" variant="outline"><CheckCircle2 />批量验活</Button><Button onClick={handleBatchForceRefresh} size="sm" variant="outline" disabled={batchRefreshing}><RefreshCw className={batchRefreshing ? 'animate-spin' : ''} />{batchRefreshing ? `刷新 ${batchRefreshProgress.current}/${batchRefreshProgress.total}` : '批量刷新'}</Button><Button onClick={handleBatchResetFailure} size="sm" variant="outline"><RotateCcw />恢复异常</Button><Button onClick={handleBatchDelete} size="sm" variant="destructive" disabled={selectedDisabledCount === 0} title={selectedDisabledCount === 0 ? '只能删除已禁用凭据' : undefined}><Trash2 />删除</Button></div></div>}
        {verifying && !verifyDialogOpen && <Button onClick={() => setVerifyDialogOpen(true)} size="sm" variant="secondary"><CheckCircle2 className="animate-spin" />验活中 {verifyProgress.current}/{verifyProgress.total}</Button>}

        {data?.credentials.length === 0 ? (
          <Card><CardContent className="p-0"><EmptyState icon={Server} title="还没有上游凭据" description="添加 Kiro OAuth 或 API Key 凭据，建立可供用户请求使用的连接池。" action={<Button onClick={() => setAddDialogOpen(true)}><Plus />添加第一个凭据</Button>} /></CardContent></Card>
        ) : filteredCredentials.length === 0 ? (
          <Card><CardContent className="p-0"><EmptyState icon={Search} title="没有匹配的凭证" description="尝试更换关键词、状态或归属筛选条件。" action={<Button variant="outline" onClick={() => { setSearch(''); setStatusFilter('all'); setOwnerFilter('all') }}>清除筛选</Button>} /></CardContent></Card>
        ) : (
          <>
            {viewMode === 'card' ? <div className="grid gap-4 md:grid-cols-2 2xl:grid-cols-3">
              {currentCredentials.map(credential => <CredentialCard key={credential.id} credential={credential} onViewBalance={handleViewBalance} selected={selectedIds.has(credential.id)} onToggleSelect={() => toggleSelect(credential.id)} balance={balanceMap.get(credential.id) || null} loadingBalance={loadingBalanceIds.has(credential.id)} />)}
            </div> : <CredentialList credentials={currentCredentials} onViewBalance={handleViewBalance} selectedIds={selectedIds} onToggleSelect={toggleSelect} balanceMap={balanceMap} loadingBalanceIds={loadingBalanceIds} />}
            {totalPages > 1 && <div className="flex flex-wrap items-center justify-center gap-3 pt-2"><Button variant="outline" size="sm" onClick={() => setCurrentPage(p => Math.max(1, p - 1))} disabled={currentPage === 1}>上一页</Button><span className="text-sm text-muted-foreground tabular-nums">第 {currentPage} / {totalPages} 页 · 共 {filteredCredentials.length} 个</span><Button variant="outline" size="sm" onClick={() => setCurrentPage(p => Math.min(totalPages, p + 1))} disabled={currentPage === totalPages}>下一页</Button></div>}
          </>
        )}
      </PageSection>

      <BalanceDialog credentialId={selectedCredentialId} open={balanceDialogOpen} onOpenChange={setBalanceDialogOpen} />
      <AddCredentialDialog open={addDialogOpen} onOpenChange={setAddDialogOpen} />
      <BatchImportDialog open={batchImportDialogOpen} onOpenChange={setBatchImportDialogOpen} />
      <KamImportDialog open={kamImportDialogOpen} onOpenChange={setKamImportDialogOpen} />
      <BatchVerifyDialog open={verifyDialogOpen} onOpenChange={setVerifyDialogOpen} verifying={verifying} progress={verifyProgress} results={verifyResults} onCancel={handleCancelVerify} />
    </div>
  )
}
