import { useState } from 'react'
import { NavLink, Outlet, useNavigate } from 'react-router-dom'
import { ChatPanel } from './ChatPanel.tsx'

const NAV = [
  { to: '/', label: 'Overview' },
  { to: '/ledger', label: 'Ledger' },
  { to: '/inventory', label: 'Inventory' },
  { to: '/processes', label: 'Processes' },
]

export function AppShell() {
  const [collapsed, setCollapsed] = useState(false)
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
    <div className="flex h-screen w-full overflow-hidden bg-slate-50 text-slate-800">
      <aside
        className={`flex-shrink-0 flex flex-col border-r border-slate-200 bg-white transition-all duration-200 ${
          collapsed ? 'w-16' : 'w-[220px]'
        }`}
      >
        <div className="flex h-14 items-center justify-between border-b border-slate-200 px-4">
          {!collapsed && (
            <span className="text-lg font-bold tracking-tight text-indigo-700">
              OpenERP
            </span>
          )}
          <button
            onClick={() => setCollapsed((c) => !c)}
            className="rounded-md p-1 text-slate-400 hover:bg-slate-100 hover:text-slate-600"
            aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
            title={collapsed ? 'Expand' : 'Collapse'}
          >
            {collapsed ? '→' : '←'}
          </button>
        </div>

        <nav className="flex-1 space-y-1 p-3">
          {NAV.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.to === '/'}
              className={({ isActive }) =>
                `flex items-center rounded-lg px-3 py-2 text-sm font-medium transition-colors ${
                  isActive
                    ? 'bg-indigo-50 text-indigo-700'
                    : 'text-slate-600 hover:bg-slate-100 hover:text-slate-900'
                }`
              }
            >
              {item.label}
            </NavLink>
          ))}
        </nav>

        <div className="border-t border-slate-200 p-3 text-xs text-slate-400">
          {!collapsed && <span>Agent-native ERP</span>}
        </div>
      </aside>

      <main className="min-w-0 flex-1 overflow-y-auto p-6">
        <Outlet />
      </main>

      <aside className="w-[380px] flex-shrink-0 flex flex-col border-l border-slate-200 bg-white">
        <ChatPanel onView={handleView} />
      </aside>
    </div>
  )
}
