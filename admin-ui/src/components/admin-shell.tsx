import { useEffect, useState } from 'react'
import { KeyRound, LogOut, Menu, Moon, Sun, X } from 'lucide-react'
import * as DialogPrimitive from '@radix-ui/react-dialog'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { sectionMeta, sectionNavItems, sectionPath, type AdminSection } from '@/lib/navigation'
export type { AdminSection } from '@/lib/navigation'

interface AdminShellProps {
  section: AdminSection
  onSectionChange: (section: AdminSection) => void
  username: string
  role: string
  isManager: boolean
  isSuperAdmin: boolean
  onLogout: () => void
  children: React.ReactNode
}

export function roleName(role: string) {
  return role === 'super_admin' ? '超级管理员' : role === 'admin' ? '管理员' : '用户'
}

export function AdminShell({ section, onSectionChange, username, role, isManager, isSuperAdmin, onLogout, children }: AdminShellProps) {
  const [dark, setDark] = useState(() => document.documentElement.classList.contains('dark'))
  const [mobileOpen, setMobileOpen] = useState(false)

  useEffect(() => {
    const saved = localStorage.getItem('kiro-admin-theme')
    const useDark = saved ? saved === 'dark' : window.matchMedia('(prefers-color-scheme: dark)').matches
    document.documentElement.classList.toggle('dark', useDark)
    setDark(useDark)
  }, [])

  const toggleTheme = () => {
    const next = !dark
    document.documentElement.classList.toggle('dark', next)
    localStorage.setItem('kiro-admin-theme', next ? 'dark' : 'light')
    setDark(next)
  }

  const navigate = (next: AdminSection) => {
    onSectionChange(next)
    setMobileOpen(false)
  }

  const navItems = sectionNavItems.filter(item => !item.requiresSuperAdmin && (!item.requiresManager || isManager) || item.requiresSuperAdmin && isSuperAdmin)
  const meta = sectionMeta[section]

  useEffect(() => {
    document.title = `${meta.title} · Kiro Console`
  }, [meta.title])

  return (
    <div className="min-h-screen overflow-x-clip bg-background text-foreground antialiased">
      <aside className="fixed inset-y-0 left-0 z-30 hidden w-[232px] flex-col border-r border-sidebar-border bg-sidebar lg:flex">
        <Brand />
        <nav className="flex-1 space-y-1 px-3 py-5" aria-label="管理导航">
          <p className="px-3 pb-2 text-[11px] font-medium uppercase tracking-[0.14em] text-muted-foreground">工作台</p>
          {navItems.map(item => <NavItem key={item.id} {...item} active={section === item.id} onClick={() => navigate(item.id)} />)}
        </nav>
        <div className="border-t border-sidebar-border p-3">
          <div className="flex items-center gap-3 rounded-xl px-2 py-2">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 font-semibold text-primary">{username.slice(0, 1).toUpperCase()}</div>
            <div className="min-w-0 flex-1"><p className="truncate text-sm font-medium">{username}</p><p className="text-xs text-muted-foreground">{roleName(role)}</p></div>
          </div>
          <Button variant="ghost" size="sm" className="mt-2 w-full justify-start text-muted-foreground" onClick={onLogout}><LogOut />退出登录</Button>
        </div>
      </aside>

      <DialogPrimitive.Root open={mobileOpen} onOpenChange={setMobileOpen}>
      {mobileOpen && <DialogPrimitive.Overlay className="fixed inset-0 z-40 bg-slate-950/30 backdrop-blur-[2px] lg:hidden" />}
      <DialogPrimitive.Content className={cn('fixed inset-y-0 left-0 z-50 flex w-[280px] flex-col bg-sidebar shadow-2xl outline-none transition-transform duration-200 ease-out lg:hidden', mobileOpen ? 'translate-x-0' : '-translate-x-full')} aria-describedby={undefined} onEscapeKeyDown={() => setMobileOpen(false)}>
        <DialogPrimitive.Title className="sr-only">管理导航</DialogPrimitive.Title>
        <div className="flex items-center justify-between border-b border-sidebar-border pr-3"><Brand /><Button variant="ghost" size="icon" aria-label="关闭导航" onClick={() => setMobileOpen(false)}><X /></Button></div>
        <nav className="flex-1 space-y-1 px-3 py-5" aria-label="移动端管理导航">
          {navItems.map(item => <NavItem key={item.id} {...item} active={section === item.id} onClick={() => navigate(item.id)} />)}
        </nav>
        <div className="border-t border-sidebar-border p-5"><p className="text-sm font-medium">{username}</p><p className="mt-1 text-xs text-muted-foreground">{roleName(role)}</p><Button variant="outline" size="sm" className="mt-4 w-full justify-start" onClick={onLogout}><LogOut />退出登录</Button></div>
      </DialogPrimitive.Content>
      </DialogPrimitive.Root>

      <div className="lg:pl-[232px]">
        <header className="sticky top-0 z-20 flex h-16 items-center justify-between border-b bg-background/90 px-4 backdrop-blur-md sm:px-6 lg:px-8">
          <div className="flex items-center gap-2"><Button variant="ghost" size="icon" className="lg:hidden" aria-label="打开导航" onClick={() => setMobileOpen(true)}><Menu /></Button><span className="text-sm font-medium text-muted-foreground">Kiro Console<span className="mx-2 text-border">/</span><span className="text-foreground">{meta.title}</span></span></div>
          <Button variant="ghost" size="icon" title={dark ? '切换浅色模式' : '切换深色模式'} aria-label={dark ? '切换浅色模式' : '切换深色模式'} onClick={toggleTheme}>{dark ? <Sun /> : <Moon />}</Button>
        </header>
        <main className="mx-auto w-full max-w-[1600px] px-4 py-7 sm:px-6 lg:px-8 lg:py-9">
          <div className="mb-7"><h1 className="text-balance text-[26px] font-semibold tracking-[-0.025em] sm:text-[28px]">{meta.title}</h1><p className="mt-2 text-pretty text-sm leading-6 text-muted-foreground">{meta.description}</p></div>
          {children}
        </main>
      </div>
    </div>
  )
}

function Brand() {
  return <div className="flex h-16 items-center gap-3 px-5"><div className="flex h-9 w-9 items-center justify-center rounded-xl bg-primary text-primary-foreground shadow-sm"><KeyRound className="h-[18px] w-[18px]" /></div><div><div className="text-[15px] font-semibold tracking-[-0.02em]">Kiro Console</div><div className="text-[11px] text-muted-foreground">服务管理中心</div></div></div>
}

function NavItem({ id, label, icon: Icon, active, onClick }: { id: AdminSection; label: string; icon: typeof import('lucide-react').Wallet; active: boolean; onClick: () => void }) {
  return <a href={sectionPath(id)} onClick={event => { if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return; event.preventDefault(); onClick() }} aria-current={active ? 'page' : undefined} className={cn('relative flex min-h-10 w-full items-center gap-3 rounded-lg px-3 text-left text-sm font-medium transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring', active ? 'bg-primary/10 text-primary' : 'text-muted-foreground hover:bg-accent hover:text-foreground')}><Icon className="h-[17px] w-[17px] shrink-0" />{label}{active && <span className="absolute right-3 h-1.5 w-1.5 rounded-full bg-primary" />}</a>
}
