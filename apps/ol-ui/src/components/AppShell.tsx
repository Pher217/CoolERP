import {
  ArrowDownLeft,
  ArrowUpRight,
  BookOpen,
  ChevronRight,
  GitBranch,
  LayoutDashboard,
  Package,
  Settings,
} from 'lucide-react'
import { NavLink, Outlet, useLocation, useNavigate } from 'react-router-dom'

import { ChatPanel } from './ChatPanel.tsx'
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
  SidebarRail,
  SidebarTrigger,
  useSidebar,
} from './ui/sidebar.tsx'

type NavLinkItem = {
  type: 'link'
  to: string
  label: string
  icon: React.ComponentType<{ className?: string }>
}

type NavGroup = {
  type: 'group'
  label: string
  icon: React.ComponentType<{ className?: string }>
  children: Array<{ to: string; label: string }>
}

type NavItem = NavLinkItem | NavGroup

const NAV: NavItem[] = [
  { type: 'link', to: '/', label: 'Overview', icon: LayoutDashboard },
  {
    type: 'group',
    label: 'General Ledger',
    icon: BookOpen,
    children: [
      { to: '/ledger', label: 'Journal Entries' },
      { to: '/accounts', label: 'Accounts' },
    ],
  },
  { type: 'link', to: '/receivables', label: 'Receivables', icon: ArrowDownLeft },
  { type: 'link', to: '/payables', label: 'Payables', icon: ArrowUpRight },
  { type: 'link', to: '/inventory', label: 'Inventory', icon: Package },
  { type: 'link', to: '/processes', label: 'Processes', icon: GitBranch },
  { type: 'link', to: '/settings', label: 'Settings', icon: Settings },
]

function isActiveRoute(pathname: string, to: string, end = false): boolean {
  if (end) return pathname === to
  return pathname === to || pathname.startsWith(`${to}/`)
}

function SidebarNavLink({
  to,
  end,
  label,
  icon: Icon,
  collapsed,
}: {
  to: string
  end?: boolean
  label: string
  icon: React.ComponentType<{ className?: string }>
  collapsed: boolean
}) {
  const { pathname } = useLocation()
  const active = isActiveRoute(pathname, to, end)

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        isActive={active}
        tooltip={collapsed ? label : undefined}
        render={
          <NavLink to={to} end={end} className="flex w-full items-center gap-2">
            <Icon className="h-4 w-4 shrink-0" />
            <span className="truncate">{label}</span>
          </NavLink>
        }
      />
    </SidebarMenuItem>
  )
}

function AppSidebar() {
  const { state } = useSidebar()
  const collapsed = state === 'collapsed'

  return (
    <Sidebar collapsible="icon" variant="sidebar" className="border-r">
      <SidebarHeader className="px-3 py-3">
        <div className="flex items-center gap-2">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-primary text-sm font-bold text-primary-foreground">
            OL
          </div>
          {!collapsed && (
            <div className="flex min-w-0 flex-col">
              <span className="truncate text-sm font-semibold leading-tight">OpenERP</span>
              <span className="text-[11px] leading-tight text-muted-foreground">Agent-native ERP</span>
            </div>
          )}
        </div>
      </SidebarHeader>

      <SidebarContent className="gap-0">
        {NAV.map((item, idx) =>
          item.type === 'link' ? (
            <SidebarMenu key={item.to} className="px-2 py-1">
              <SidebarNavLink
                to={item.to}
                end={item.to === '/'}
                label={item.label}
                icon={item.icon}
                collapsed={collapsed}
              />
            </SidebarMenu>
          ) : (
            <SidebarGroup key={idx} className="py-1">
              {!collapsed && <SidebarGroupLabel>{item.label}</SidebarGroupLabel>}
              <SidebarGroupContent>
                <SidebarMenu className="px-0">
                  {item.children.map((child) => (
                    <SidebarNavLink
                      key={child.to}
                      to={child.to}
                      label={child.label}
                      icon={item.icon}
                      collapsed={collapsed}
                    />
                  ))}
                </SidebarMenu>
              </SidebarGroupContent>
            </SidebarGroup>
          ),
        )}
      </SidebarContent>

      <SidebarFooter className="px-2 py-2">
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarTrigger />
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  )
}

function routeLabel(pathname: string): string {
  if (pathname === '/') return 'Overview'
  if (pathname.startsWith('/ledger')) return 'Journal Entries'
  if (pathname.startsWith('/accounts')) return 'Accounts'
  if (pathname.startsWith('/receivables')) return 'Receivables'
  if (pathname.startsWith('/payables')) return 'Payables'
  if (pathname.startsWith('/inventory')) return 'Inventory'
  if (pathname.startsWith('/processes')) return 'Processes'
  if (pathname.startsWith('/settings')) return 'Settings'
  return 'OpenERP'
}

function BreadcrumbBar() {
  const { pathname } = useLocation()
  const label = routeLabel(pathname)

  return (
    <header className="sticky top-0 z-10 flex h-14 items-center gap-2 border-b bg-background/95 px-4 backdrop-blur">
      <SidebarTrigger className="-ml-1 md:hidden" />
      <div className="flex items-center gap-1 text-sm text-muted-foreground">
        <span>OpenERP</span>
        <ChevronRight className="h-4 w-4" />
        <span className="font-medium text-foreground">{label}</span>
      </div>
    </header>
  )
}

export function AppShell() {
  const navigate = useNavigate()

  function handleView(module: string, _focus?: string | null) {
    const route =
      module === 'ledger'
        ? '/ledger'
        : module === 'inventory'
          ? '/inventory'
          : module === 'workflows'
            ? '/processes'
            : '/'
    navigate(route)
  }

  return (
    <SidebarProvider defaultOpen={true}>
      <div className="flex h-screen w-full overflow-hidden bg-background">
        <AppSidebar />

        <div className="flex min-w-0 flex-1 overflow-hidden">
          <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
            <BreadcrumbBar />
            <div className="flex-1 overflow-y-auto p-6">
              <Outlet />
            </div>
          </main>

          <aside className="hidden w-[360px] flex-shrink-0 flex-col border-l border-l-border/70 bg-muted/30 lg:flex">
            <div className="flex h-14 flex-shrink-0 items-center gap-2 border-b px-4">
              <div className="flex h-6 w-6 items-center justify-center rounded-md bg-primary/10">
                <svg className="h-3.5 w-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M8 10h.01M12 10h.01M16 10h.01M9 16H5a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v8a2 2 0 01-2 2h-5l-5 5v-5z" />
                </svg>
              </div>
              <div>
                <h2 className="text-sm font-semibold leading-none">AI Assistant</h2>
                <p className="mt-0.5 text-[11px] text-muted-foreground">Powered by ol-api</p>
              </div>
            </div>
            <div className="flex-1 overflow-hidden">
              <ChatPanel onView={handleView} />
            </div>
          </aside>
        </div>
      </div>
    </SidebarProvider>
  )
}
