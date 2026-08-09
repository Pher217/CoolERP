import { useEffect, useState } from 'react'
import { RefreshCw } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { api, type InventoryItem } from '../api/client.ts'

type LoadState =
  | { status: 'loading' }
  | { status: 'ok'; items: InventoryItem[] }
  | { status: 'error' }

export function InventoryPanel() {
  const [state, setState] = useState<LoadState>({ status: 'loading' })

  // Manual refresh: reset to 'loading' first, since there is already content
  // on screen that the user asked to replace.
  function reload() {
    setState({ status: 'loading' })
    void api
      .inventory()
      .then((resp) => setState({ status: 'ok', items: resp.items }))
      .catch(() => setState({ status: 'error' }))
  }

  // Mount fetch. State already starts as 'loading', so this must not set it
  // again synchronously. `cancelled` guards against a resolve landing after
  // unmount (or after a fast remount in StrictMode).
  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const resp = await api.inventory()
        if (!cancelled) setState({ status: 'ok', items: resp.items })
      } catch {
        if (!cancelled) setState({ status: 'error' })
      }
    })()
    return () => {
      cancelled = true
    }
  }, [])

  return (
    <div className="mx-auto max-w-6xl space-y-6">
      <h1 className="text-2xl font-bold tracking-tight text-foreground">Inventory</h1>

      <Card className="shadow-sm">
        <CardHeader className="flex flex-row items-center justify-between">
          <div>
            <CardTitle>Stock on hand</CardTitle>
            <CardDescription>Live inventory quantities from ol-api.</CardDescription>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={() => reload()}
            disabled={state.status === 'loading'}
            aria-label="Refresh inventory"
          >
            <RefreshCw className={`mr-2 h-4 w-4 ${state.status === 'loading' ? 'animate-spin' : ''}`} />
            {state.status === 'loading' ? 'Loading…' : 'Refresh'}
          </Button>
        </CardHeader>
        <CardContent className="p-0">
          {state.status === 'error' ? (
            <div className="px-6 py-8 text-center text-sm text-muted-foreground">
              API offline — start ol-api to load inventory.
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>SKU</TableHead>
                  <TableHead>Name</TableHead>
                  <TableHead className="text-right">On hand</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {state.status === 'loading' && (
                  <TableRow>
                    <TableCell colSpan={3}>
                      <Skeleton className="h-4 w-full" />
                    </TableCell>
                  </TableRow>
                )}
                {state.status === 'ok' && state.items.length === 0 && (
                  <TableRow>
                    <TableCell colSpan={3} className="text-center text-muted-foreground">
                      No inventory items.
                    </TableCell>
                  </TableRow>
                )}
                {state.status === 'ok' &&
                  state.items.map((item) => (
                    <TableRow key={item.sku}>
                      <TableCell className="font-medium">{item.sku}</TableCell>
                      <TableCell>{item.name}</TableCell>
                      <TableCell className="text-right tabular-nums">{item.on_hand}</TableCell>
                    </TableRow>
                  ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
