import { useEffect, useRef, useState } from 'react'
import mermaid from 'mermaid'
import { api, type ProcessDetailResponse } from '../api/client.ts'

mermaid.initialize({ startOnLoad: false, theme: 'neutral' })

type LoadState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ok'; data: ProcessDetailResponse }
  | { status: 'error'; message: string }
  | { status: 'offline' }

let mermaidIdCounter = 0

function MermaidDiagram({ chart }: { chart: string }) {
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!ref.current) return
    const id = `mermaid-${++mermaidIdCounter}`
    mermaid
      .render(id, chart)
      .then(({ svg }) => {
        if (ref.current) ref.current.innerHTML = svg
      })
      .catch(() => {
        if (ref.current) ref.current.textContent = 'Diagram render failed.'
      })
  }, [chart])

  return <div ref={ref} className="mermaid-container" />
}

export function WorkflowView() {
  const [processes, setProcesses] = useState<string[]>([])
  const [selected, setSelected] = useState<string>('customer_invoice')
  const [detail, setDetail] = useState<LoadState>({ status: 'idle' })
  const [listOffline, setListOffline] = useState(false)

  // Load process list once
  useEffect(() => {
    api
      .listProcesses()
      .then((r) => {
        setProcesses(r.processes)
        // Keep default selection if present, else use first
        if (!r.processes.includes(selected) && r.processes.length > 0) {
          setSelected(r.processes[0])
        }
      })
      .catch(() => setListOffline(true))
  }, []) // eslint-disable-line react-hooks/exhaustive-deps

  // Load detail whenever selection changes
  useEffect(() => {
    if (!selected) return
    let cancelled = false

    async function fetchDetail() {
      setDetail({ status: 'loading' })
      try {
        const data = await api.getProcess(selected)
        if (!cancelled) setDetail({ status: 'ok', data })
      } catch (err: unknown) {
        if (cancelled) return
        const msg = err instanceof Error ? err.message : String(err)
        if (msg.includes('Failed to fetch') || msg.includes('NetworkError')) {
          setDetail({ status: 'offline' })
        } else {
          setDetail({ status: 'error', message: msg })
        }
      }
    }

    void fetchDetail()
    return () => { cancelled = true }
  }, [selected])

  return (
    <div className="widget">
      <h2>Workflows</h2>

      {listOffline ? (
        <p className="muted">API offline — start ol-api to load workflows.</p>
      ) : (
        <>
          <div className="workflow-selector">
            <label htmlFor="process-select">Process:</label>
            <select
              id="process-select"
              value={selected}
              onChange={(e) => setSelected(e.target.value)}
            >
              {processes.length === 0 && (
                <option value="customer_invoice">customer_invoice</option>
              )}
              {processes.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
          </div>

          {detail.status === 'loading' && <p className="muted">Loading…</p>}
          {detail.status === 'offline' && (
            <p className="muted">API offline — start ol-api to load process detail.</p>
          )}
          {detail.status === 'error' && (
            <p className="error-text">Error: {detail.message}</p>
          )}
          {detail.status === 'ok' && (
            <div className="workflow-detail">
              <div className="workflow-meta">
                <div className="workflow-section">
                  <h3>States</h3>
                  <ul className="tag-list">
                    {detail.data.states.map((s) => (
                      <li key={s} className="tag">
                        {s}
                      </li>
                    ))}
                  </ul>
                </div>
                <div className="workflow-section">
                  <h3>Transitions</h3>
                  <table className="transitions-table">
                    <thead>
                      <tr>
                        <th>From</th>
                        <th>To</th>
                        <th>Capability</th>
                      </tr>
                    </thead>
                    <tbody>
                      {detail.data.transitions.map((t, i) => (
                        <tr key={i}>
                          <td>{t.from}</td>
                          <td>{t.to}</td>
                          <td>{t.capability ?? <span className="muted">—</span>}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
              <div className="workflow-diagram">
                <h3>Diagram</h3>
                <MermaidDiagram chart={detail.data.mermaid} />
              </div>
            </div>
          )}
        </>
      )}
    </div>
  )
}
