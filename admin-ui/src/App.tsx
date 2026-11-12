import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { storage } from '@/lib/storage'
import { getAccount, logoutAccount } from '@/api/accounts'
import { LoginPage } from '@/components/login-page'
import { Dashboard } from '@/components/dashboard'
import { AccountDashboard } from '@/components/account-dashboard'
import { Toaster } from '@/components/ui/sonner'
import type { AccountOverview } from '@/types/accounts'

function App() {
  const [account, setAccount] = useState<AccountOverview | null>(null)
  const [mode, setMode] = useState<'legacy' | 'accounts'>('legacy')
  const [showLogin, setShowLogin] = useState(false)

  const queryClient = useQueryClient()
  const accountQuery = useQuery({
    queryKey: ['account-session'],
    queryFn: getAccount,
    enabled: Boolean(storage.getApiKey()) && !account && !showLogin,
    retry: false,
  })
  if (accountQuery.data && !account && mode === 'legacy') {
    setAccount(accountQuery.data)
    if (accountQuery.data.user.role === 'user') setMode('accounts')
  }
  if (accountQuery.isError && !account) {
    storage.removeApiKey()
    queryClient.removeQueries({ queryKey: ['account-session'] })
    setShowLogin(true)
  }

  const logout = () => {
    void logoutAccount().catch(() => undefined)
    storage.removeApiKey()
    queryClient.clear()
    setAccount(null)
    setMode('legacy')
    setShowLogin(true)
  }

  return (
    <>
      {account && mode === 'accounts' ? (
        <div className="min-h-screen bg-background"><main className="container mx-auto px-4 md:px-8 py-6"><AccountDashboard initialAccount={account} onLogout={logout} onOpenCredentials={() => setMode('legacy')} /></main></div>
      ) : account && mode === 'legacy' ? (
        <Dashboard onLogout={logout} onOpenAccounts={() => setMode('accounts')} />
      ) : (
        <LoginPage onLogin={result => { setAccount(result); setMode(result.user.role === 'user' ? 'accounts' : 'legacy'); setShowLogin(false) }} />
      )}
      <Toaster position="top-right" />
    </>
  )
}

export default App
