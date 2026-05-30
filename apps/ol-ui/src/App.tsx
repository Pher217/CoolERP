import { useEffect, useState } from 'react'
import { api, type HealthResponse, type BalanceResponse } from './api/client.ts'
import { formatMoney } from './lib/money.ts'
import './App.css'

// Seeded chart-of-accounts codes (from migrations/seed)
const SEEDED_ACCOUNTS = ['1000', '4000', '2000', '3000', '5000']

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

type AccountRow = {
  code: string
  balance: BalanceResponse | null
  error: boolean
}

function AccountsPanel() {
  const [rows, setRows] = useState<AccountRow[]>(
    SEEDED_ACCOUNTS.map((code) => ({ code, balance: null, error: false })),
  )
  const [apiOffline, setApiOffline] = useState(false)

  useEffect(() => {
    let cancelled = false

    Promise.all(
      SEEDED_ACCOUNTS.map((code) =>
        api
          .accountBalance(code)
          .then((b) => ({ code, balance: b, error: false }))
          .catch(() => ({ code, balance: null, error: true })),
      ),
    ).then((results) => {
      if (cancelled) return
      // If every account failed, assume API is offline
      if (results.every((r) => r.error)) {
        setApiOffline(true)
      } else {
        setRows(results)
      }
    })

    return () => {
      cancelled = true
    }
  }, [])

  if (apiOffline) {
    return (
      <div className="widget">
        <h2>Accounts</h2>
        <p className="muted">API offline — start ol-api to load balances.</p>
      </div>
    )
  }

  return (
    <div className="widget">
      <h2>Accounts</h2>
      <table className="accounts-table">
        <thead>
          <tr>
            <th>Code</th>
            <th>Balance</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ code, balance, error }) => (
            <tr key={code}>
              <td>{code}</td>
              <td>
                {error ? (
                  <span className="muted">—</span>
                ) : balance === null ? (
                  <span className="muted">Loading…</span>
                ) : (
                  formatMoney(balance.balance_cents, balance.currency ?? 'USD', 'en-US')
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

export default function App() {
  return (
    <div className="app">
      <header className="app-header">
        <span className="logo">OpenLedger</span>
        <span className="tagline">Correct by construction · Agent-native by design</span>
      </header>
      <main className="dashboard">
        <BackendStatusWidget />
        <AccountsPanel />
      </main>
    </div>
  )
}
