import { useEffect, useRef, useState } from 'react'
import mermaid from 'mermaid'

import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { Skeleton } from '@/components/ui/skeleton'
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

  return <div ref={ref} className="overflow-x-auto rounded-lg border bg-muted/30 p-4" />
}

export function WorkflowView() {
  const [processes, setProcesses] = useState<string[]>([])
  const [selected, setSelected] = useState<string>('customer_invoice')
  const [detail, setDetail] = useState<LoadState>({ status: 'idle' })
  const [listOffline, setListOffline] = useState(false)

  useEffect(() => {
    api
      .listProcesses()
      .then((r) => {
        setProcesses(r.processes)
        if (!r.processes.includes(selected) && r.processes.length > 0) {
          setSelected(r.processes[0])
        }
      })
      .catch(() => setListOffline(true))
  }, []) // eslint-disable-line react-hooks/exhaustive-deps

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
    return () => {
      cancelled = true
    }
  }, [selected])

  return (
    <div className="mx-auto max-w-6xl space-y-6">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <h1 className="text-2xl font-bold tracking-tight text-foreground">Processes</h1>
        {!listOffline && (
          <div className="flex items-center gap-2">
            <span className="text-sm text-muted-foreground">Process</span>
            <Select
              value={selected}
              onValueChange={(value) => value && setSelected(value)}
            >
              <SelectTrigger className="w-[220px]">
                <SelectValue placeholder="Select process" />
              </SelectTrigger>
              <SelectContent>
                {processes.length === 0 && (
                  <SelectItem value="customer_invoice">customer_invoice</SelectItem>
                )}
                {processes.map((p) => (
                  <SelectItem key={p} value={p}>
                    {p}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}
      </div>

      {listOffline ? (
        <Card className="shadow-sm">
          <CardHeader>
            <CardTitle>Workflows</CardTitle>
            <CardDescription>API offline — start ol-api to load workflows.</CardDescription>
          </CardHeader>
        </Card>
      ) : (
        <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
          <Card className="shadow-sm lg:col-span-1">
            <CardHeader>
              <CardTitle>States</CardTitle>
              <CardDescription>Defined states for {selected}</CardDescription>
            </CardHeader>
            <CardContent>
              {detail.status === 'loading' && <Skeleton className="h-4 w-full" />}
              {detail.status === 'offline' && (
                <p className="text-sm text-muted-foreground">API offline.</p>
              )}
              {detail.status === 'error' && (
                <p className="text-sm text-destructive">Error: {detail.message}</p>
              )}
              {detail.status === 'ok' && (
                <div className="flex flex-wrap gap-2">
                  {detail.data.states.map((s) => (
                    <Badge key={s} variant="secondary">
                      {s}
                    </Badge>
                  ))}
                </div>
              )}
            </CardContent>
          </Card>

          <Card className="shadow-sm lg:col-span-2">
            <CardHeader>
              <CardTitle>Transitions</CardTitle>
              <CardDescription>Capabilities required for each transition</CardDescription>
            </CardHeader>
            <CardContent className="p-0">
              {detail.status === 'loading' && (
                <div className="px-6 py-4">
                  <Skeleton className="h-4 w-full" />
                </div>
              )}
              {detail.status === 'offline' && (
                <div className="px-6 py-8 text-center text-sm text-muted-foreground">
                  API offline — start ol-api to load process detail.
                </div>
              )}
              {detail.status === 'error' && (
                <div className="px-6 py-8 text-center text-sm text-destructive">
                  Error: {detail.message}
                </div>
              )}
              {detail.status === 'ok' && (
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>From</TableHead>
                      <TableHead>To</TableHead>
                      <TableHead>Capability</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {detail.data.transitions.map((t, i) => (
                      <TableRow key={i}>
                        <TableCell className="font-medium">{t.from}</TableCell>
                        <TableCell>{t.to}</TableCell>
                        <TableCell>
                          {t.capability ?? (
                            <span className="text-muted-foreground">—</span>
                          )}
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              )}
            </CardContent>
          </Card>

          <Card className="shadow-sm lg:col-span-3">
            <CardHeader>
              <CardTitle>Diagram</CardTitle>
            </CardHeader>
            <CardContent>
              {detail.status === 'loading' && <Skeleton className="h-48 w-full" />}
              {detail.status === 'offline' && (
                <p className="text-sm text-muted-foreground">API offline.</p>
              )}
              {detail.status === 'error' && (
                <p className="text-sm text-destructive">Error: {detail.message}</p>
              )}
              {detail.status === 'ok' && <MermaidDiagram chart={detail.data.mermaid} />}
            </CardContent>
          </Card>
        </div>
      )}
    </div>
  )
}
