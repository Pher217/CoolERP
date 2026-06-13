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
      <table className="w-full border-collapse text-sm">
        <thead>
          <tr className="border-b border-slate-200 text-left text-xs font-semibold uppercase tracking-wide text-slate-500">
            <th className="py-2 pr-2">SKU</th>
            <th className="py-2 px-2">Name</th>
            <th className="py-2 pl-2 text-right">On hand</th>
          </tr>
        </thead>
        <tbody className="text-slate-700">
          {state.status === 'ok' && state.items.length === 0 && (
            <tr>
              <td colSpan={3} className="py-2 text-slate-400">
                No inventory items.
              </td>
            </tr>
          )}
          {state.status === 'ok' &&
            state.items.map((item) => (
              <tr
                key={item.sku}
                className="border-b border-slate-100 transition-colors hover:bg-slate-50"
              >
                <td className="py-2 pr-2">{item.sku}</td>
                <td className="py-2 px-2">{item.name}</td>
                <td className="py-2 pl-2 text-right tabular-nums">{item.on_hand}</td>
              </tr>
            ))}
          {state.status === 'loading' && (
            <tr>
              <td colSpan={3} className="py-2 text-slate-400">
                Loading…
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  )
}
