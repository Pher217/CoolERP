import { useCallback, useEffect, useRef, useState } from 'react'
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
import { api, type BalanceResponse } from '../api/client.ts'
import { formatMoney } from '../lib/money.ts'

const SEEDED_ACCOUNTS = ['1000', '4000', '2000', '3000', '5000']

type AccountRow = {
  code: string
  balance: BalanceResponse | null
  error: boolean
}

type Props = {
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

  useEffect(() => {
    onRegisterRefresh?.(triggerRefresh)
  }, [triggerRefresh, onRegisterRefresh])

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
    return () => {
      cancelled = true
    }
  }, [loading])

  if (apiOffline) {
    return (
      <Card className="shadow-sm">
        <CardHeader>
          <CardTitle>Accounts</CardTitle>
          <CardDescription>API offline — start ol-api to load balances.</CardDescription>
        </CardHeader>
      </Card>
    )
  }

  return (
    <Card className="shadow-sm">
      <CardHeader className="flex flex-row items-center justify-between">
        <div>
          <CardTitle>Accounts</CardTitle>
          <CardDescription>Live balances for seeded chart-of-accounts codes.</CardDescription>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={triggerRefresh}
          disabled={loading}
          aria-label="Refresh balances"
        >
          <RefreshCw className={`mr-2 h-4 w-4 ${loading ? 'animate-spin' : ''}`} />
          {loading ? 'Loading…' : 'Refresh'}
        </Button>
      </CardHeader>
      <CardContent className="p-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-32">Code</TableHead>
              <TableHead className="text-right">Balance</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map(({ code, balance, error }) => (
              <TableRow key={code}>
                <TableCell className="font-medium">{code}</TableCell>
                <TableCell className="text-right tabular-nums">
                  {error ? (
                    <span className="text-muted-foreground">—</span>
                  ) : balance === null || loading ? (
                    <Skeleton className="ml-auto h-4 w-24" />
                  ) : (
                    formatMoney(balance.balance, balance.currency, 'en-US')
                  )}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  )
}
