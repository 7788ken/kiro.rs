import { useEffect, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Loader2 } from 'lucide-react'
import { getAccount, logoutAccount } from '@/api/accounts'
import { storage } from '@/lib/storage'
import { LoginPage } from '@/components/login-page'
import { Dashboard } from '@/components/dashboard'
import { AccountDashboard } from '@/components/account-dashboard'
import { AdminShell, type AdminSection } from '@/components/admin-shell'
import { Toaster } from '@/components/ui/sonner'
import { navigateTo, replaceLocation, sectionPath, useAdminSection } from '@/lib/navigation'
import type { AccountOverview } from '@/types/accounts'

function defaultSection(account: AccountOverview): AdminSection {
  return account.user.role === 'user' ? 'overview' : 'credentials'
}

function App() {
  const [account, setAccount] = useState<AccountOverview | null>(null)
  const [showLogin, setShowLogin] = useState(false)
  const [requestedPath, setRequestedPath] = useState<string | null>(null)
  const section = useAdminSection()
  const queryClient = useQueryClient()
  const accountQuery = useQuery({
    queryKey: ['account-session'],
    queryFn: getAccount,
    enabled: Boolean(storage.getApiKey()) && !account && !showLogin,
    retry: false,
  })

  useEffect(() => {
    if (accountQuery.data && !account) {
      setAccount(accountQuery.data)
      if (!section) navigateTo(sectionPath(defaultSection(accountQuery.data)), true)
    }
    if (accountQuery.isError && !account) {
      storage.removeApiKey()
      queryClient.removeQueries({ queryKey: ['account-session'] })
      setShowLogin(true)
    }
  }, [account, accountQuery.data, accountQuery.isError, queryClient, section])

  useEffect(() => {
    if (!account || section) return
    navigateTo(sectionPath(defaultSection(account)), true)
  }, [account, section])

  const logout = () => {
    void logoutAccount().catch(() => undefined)
    storage.removeApiKey()
    queryClient.clear()
    setAccount(null)
    setShowLogin(true)
    replaceLocation('/admin')
  }

  const login = (result: AccountOverview) => {
    setAccount(result)
    setShowLogin(false)
    if (requestedPath) {
      navigateTo(requestedPath, true)
      setRequestedPath(null)
    } else if (!section) {
      navigateTo(sectionPath(defaultSection(result)), true)
    }
  }

  const role = accountQuery.data?.user.role || account?.user.role
  const accessible = section && (section !== 'credentials' || role === 'super_admin') && (section !== 'users' || role !== 'user')

  useEffect(() => {
    if (section && role && !accessible) navigateTo(sectionPath('overview'), true)
  }, [section, role, accessible])

  return (
    <>
      {account && section ? (        <AdminShell
          section={section}
          onSectionChange={next => navigateTo(sectionPath(next))}
          username={accountQuery.data?.user.username || account.user.username}
          role={role || account.user.role}
          isManager={role !== 'user'}
          isSuperAdmin={role === 'super_admin'}
          onLogout={logout}
        >
          {!accessible ? null : section === 'credentials' ? (
            <Dashboard onLogout={logout} />
          ) : section === 'users' ? (
            <AccountDashboard initialAccount={account} section={section} onSectionChange={next => navigateTo(sectionPath(next))} onLogout={logout} />
          ) : (
            <AccountDashboard initialAccount={account} section={section} onSectionChange={next => navigateTo(sectionPath(next))} onLogout={logout} />
          )}
        </AdminShell>
      ) : account || accountQuery.isLoading || accountQuery.isFetching && !showLogin ? (
        <div className="flex min-h-screen items-center justify-center bg-background" role="status"><Loader2 className="h-6 w-6 animate-spin text-primary" /><span className="sr-only">正在加载管理后台</span></div>
      ) : (
        <LoginPage onLogin={login} />
      )}
      <Toaster position="top-right" />
    </>
  )
}

export default App
