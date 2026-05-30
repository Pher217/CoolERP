import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type BalanceResponse } from '../api/client.ts'
import { formatMoney } from '../lib/money.ts'

// Seeded chart-of-accounts codes (from migrations/seed)
const SEEDED_ACCOUNTS = ['1000', '4000', '2000', '3000', '5000']

type AccountRow = {
  code: string
  balance: BalanceResponse | null
  error: boolean
}

type Props = {
  /** Called once on mount with the refresh function so a parent can trigger a reload. */
  onRegisterRefresh?: (fn: () => void) => void
}

export function AccountsPanel({ onRegisterRefresh }: Props) {
  const [rows, setRows] = useState<AccountRow[]>(
    SEEDED_ACCOUNTS.map((code) => ({ code, balance: null, error: false })),
  )
  const [apiOffline, setApiOffline] = useState(false)
  const [loading, setLoading] = useState(true)
  const refreshTrigger = useRef(0)

  const triggerRefresh = useCallback(() => {
    refreshTrigger.current += 1
    setLoading(true)
  }, [])

  // Register the refresh handle once on mount.
  useEffect(() => {
    onRegisterRefresh?.(triggerRefresh)
  }, [triggerRefresh, onRegisterRefresh])

  // Fetch balances whenever loading transitions to true.
  useEffect(() => {
    if (!loading) return
    let cancelled = false

    async function fetchAll() {
      const results = await Promise.all(
        SEEDED_ACCOUNTS.map((code) =>
          api
            .accountBalance(code)
            .then((b) => ({ code, balance: b, error: false }))
            .catch(() => ({ code, balance: null as BalanceResponse | null, error: true })),
        ),
      )
      if (cancelled) return
      if (results.every((r) => r.error)) {
        setApiOffline(true)
      } else {
        setApiOffline(false)
        setRows(results)
      }
      setLoading(false)
    }

    void fetchAll()
    return () => { cancelled = true }
  }, [loading])

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
      <div className="widget-header">
        <h2>Accounts</h2>
        <button
          className="btn-secondary btn-sm"
          onClick={triggerRefresh}
          disabled={loading}
          aria-label="Refresh balances"
        >
          {loading ? 'Loading…' : 'Refresh'}
        </button>
      </div>
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
                  formatMoney(balance.balance, 'EUR', 'en-US')
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
