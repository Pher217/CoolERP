import { useEffect, useState } from 'react'
import { api, type InventoryItem } from '../api/client.ts'

type LoadState =
  | { status: 'loading' }
  | { status: 'ok'; items: InventoryItem[] }
  | { status: 'error' }

export function InventoryPanel() {
  const [state, setState] = useState<LoadState>({ status: 'loading' })

  async function load() {
    setState({ status: 'loading' })
    try {
      const resp = await api.inventory()
      setState({ status: 'ok', items: resp.items })
    } catch {
      setState({ status: 'error' })
    }
  }

  useEffect(() => {
    void load()
  }, [])

  if (state.status === 'error') {
    return (
      <div className="widget">
        <h2>Inventory</h2>
        <p className="muted">API offline — start ol-api to load inventory.</p>
      </div>
    )
  }

  return (
    <div className="widget">
      <div className="widget-header">
        <h2>Inventory</h2>
        <button
          className="btn-secondary btn-sm"
          onClick={() => void load()}
          disabled={state.status === 'loading'}
          aria-label="Refresh inventory"
        >
          {state.status === 'loading' ? 'Loading…' : 'Refresh'}
        </button>
      </div>
      <table className="inventory-table">
        <thead>
          <tr>
            <th>SKU</th>
            <th>Name</th>
            <th>On hand</th>
          </tr>
        </thead>
        <tbody>
          {state.status === 'ok' && state.items.length === 0 && (
            <tr>
              <td colSpan={3}>
                <span className="muted">No inventory items.</span>
              </td>
            </tr>
          )}
          {state.status === 'ok' &&
            state.items.map((item) => (
              <tr key={item.sku}>
                <td>{item.sku}</td>
                <td>{item.name}</td>
                <td>{item.on_hand}</td>
              </tr>
            ))}
          {state.status === 'loading' && (
            <tr>
              <td colSpan={3}>
                <span className="muted">Loading…</span>
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  )
}
