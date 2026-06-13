import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type HealthResponse } from './api/client.ts'
import { AccountsPanel } from './components/AccountsPanel.tsx'
import { WorkflowView } from './components/WorkflowView.tsx'
import { PostEntryForm } from './components/PostEntryForm.tsx'
import { ChatPanel } from './components/ChatPanel.tsx'
import './App.css'

type BackendStatus = 'checking' | 'ok' | 'unreachable'

function BackendStatusWidget() {
  const [status, setStatus] = useState<BackendStatus>('checking')
  const [detail, setDetail] = useState<HealthResponse | null>(null)

  useEffect(() => {
    api
      .health()
      .then((r) => {
        setDetail(r)
        setStatus('ok')
      })
      .catch(() => setStatus('unreachable'))
  }, [])

  const badge =
    status === 'checking'
      ? { label: 'Checking…', color: '#94a3b8' }
      : status === 'ok'
        ? { label: 'Online', color: '#22c55e' }
        : { label: 'Unreachable', color: '#ef4444' }

  return (
    <div className="widget">
      <h2>Backend status</h2>
      <span className="badge" style={{ backgroundColor: badge.color }}>
        {badge.label}
      </span>
      {detail && (
        <p className="detail">
          API response: <code>{JSON.stringify(detail)}</code>
        </p>
      )}
      {status === 'unreachable' && (
        <p className="detail muted">
          Start ol-api on {import.meta.env.VITE_API_BASE ?? 'http://localhost:3000'} to connect.
        </p>
      )}
    </div>
  )
}

export default function App() {
  // accountsPanelRef allows PostEntryForm to trigger a balances refresh
  const refreshBalancesRef = useRef<() => void>(() => {})

  const registerRefresh = useCallback((fn: () => void) => {
    refreshBalancesRef.current = fn
  }, [])

  return (
    <div className="app">
      <header className="app-header">
        <span className="logo">OpenERP</span>
        <span className="tagline">Correct by construction · Agent-native by design</span>
      </header>
      <main className="dashboard">
        <BackendStatusWidget />
        <AccountsPanel onRegisterRefresh={registerRefresh} />
        <PostEntryForm onSuccess={() => refreshBalancesRef.current()} />
        <WorkflowView />
        <ChatPanel />
      </main>
    </div>
  )
}
