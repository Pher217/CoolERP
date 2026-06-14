import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import {
  Bar,
  BarChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'

import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { api, type BalanceResponse, type InventoryResponse } from '../api/client.ts'
import { formatMoney } from '../lib/money.ts'
import { KpiCard } from '../components/KpiCard.tsx'

type LoadState<T> = {
  data: T | null
  error: boolean
}

function emDash(): string {
  return '—'
}

export function Overview() {
  const navigate = useNavigate()

  const [cash, setCash] = useState<LoadState<BalanceResponse>>({ data: null, error: false })
  const [revenue, setRevenue] = useState<LoadState<BalanceResponse>>({
    data: null,
    error: false,
  })
  const [expenses, setExpenses] = useState<LoadState<BalanceResponse>>({
    data: null,
    error: false,
  })
  const [inventory, setInventory] = useState<LoadState<InventoryResponse>>({
    data: null,
    error: false,
  })

  useEffect(() => {
    let cancelled = false

    async function load() {
      try {
        const [cashResp, revenueResp, expensesResp] = await Promise.all([
          api.accountBalance('1000'),
          api.accountBalance('4000'),
          api.accountBalance('5000'),
        ])
        if (cancelled) return
        setCash({ data: cashResp, error: false })
        setRevenue({ data: revenueResp, error: false })
        setExpenses({ data: expensesResp, error: false })
      } catch {
        if (cancelled) return
        setCash({ data: null, error: true })
        setRevenue({ data: null, error: true })
        setExpenses({ data: null, error: true })
      }

      try {
        const invResp = await api.inventory()
        if (cancelled) return
        setInventory({ data: invResp, error: false })
      } catch {
        if (cancelled) return
        setInventory({ data: null, error: true })
      }
    }

    void load()
    return () => {
      cancelled = true
    }
  }, [])

  const cashValue = cash.error || !cash.data ? emDash() : formatMoney(cash.data.balance, 'EUR', 'en-US')
  const revenueValue =
    revenue.error || !revenue.data ? emDash() : formatMoney(revenue.data.balance, 'EUR', 'en-US')
  const expensesValue =
    expenses.error || !expenses.data ? emDash() : formatMoney(expenses.data.balance, 'EUR', 'en-US')

  const inventoryValue =
    inventory.error || !inventory.data ? emDash() : String(inventory.data.items.length)
  const totalOnHand =
    inventory.error || !inventory.data
      ? null
      : inventory.data.items.reduce((sum, item) => sum + Number(item.on_hand), 0)
  const inventorySubText = totalOnHand === null ? undefined : `${totalOnHand} units on hand`

  const chartData = [
    {
      name: 'Cash',
      value: cash.error || !cash.data ? 0 : cash.data.balance / 100,
    },
    {
      name: 'Revenue',
      value: revenue.error || !revenue.data ? 0 : revenue.data.balance / 100,
    },
    {
      name: 'Expenses',
      value: expenses.error || !expenses.data ? 0 : expenses.data.balance / 100,
    },
  ]

  const outOfStock =
    inventory.error || !inventory.data
      ? []
      : inventory.data.items.filter((item) => Number(item.on_hand) === 0)

  return (
    <div className="mx-auto max-w-6xl space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight text-foreground">Overview</h1>
        <p className="mt-1 text-sm text-muted-foreground">Financial snapshot and key metrics</p>
      </div>

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <KpiCard label="Cash on hand" value={cashValue} subText="Account 1000" />
        <KpiCard label="Revenue" value={revenueValue} subText="Account 4000" />
        <KpiCard label="Expenses" value={expensesValue} subText="Account 5000" />
        <KpiCard label="Inventory SKUs" value={inventoryValue} subText={inventorySubText} />
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle className="text-sm font-semibold uppercase tracking-widest text-muted-foreground">
              Balances
            </CardTitle>
          </CardHeader>
          <CardContent className="pt-2">
            <div className="h-72">
              <ResponsiveContainer width="100%" height="100%">
                <BarChart data={chartData} margin={{ top: 8, right: 16, bottom: 8, left: 0 }}>
                  <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" vertical={false} />
                  <XAxis
                    dataKey="name"
                    tick={{ fill: 'var(--muted-foreground)', fontSize: 12 }}
                    axisLine={false}
                    tickLine={false}
                  />
                  <YAxis
                    tick={{ fill: 'var(--muted-foreground)', fontSize: 12 }}
                    axisLine={false}
                    tickLine={false}
                    width={70}
                    tickFormatter={(v) =>
                      new Intl.NumberFormat('en-US', {
                        notation: 'compact',
                        style: 'currency',
                        currency: 'EUR',
                      }).format(typeof v === 'number' ? v : 0)
                    }
                  />
                  <Tooltip
                    cursor={{ fill: 'var(--muted)', opacity: 0.5 }}
                    contentStyle={{
                      background: 'var(--card)',
                      border: '1px solid var(--border)',
                      borderRadius: '0.5rem',
                      fontSize: '12px',
                      color: 'var(--foreground)',
                    }}
                    formatter={(value) => [
                      new Intl.NumberFormat('en-US', {
                        style: 'currency',
                        currency: 'EUR',
                      }).format(typeof value === 'number' ? value : 0),
                      'Balance',
                    ]}
                  />
                  <Bar dataKey="value" fill="var(--chart-1)" radius={[4, 4, 0, 0]} />
                </BarChart>
              </ResponsiveContainer>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-sm font-semibold uppercase tracking-widest text-muted-foreground">
              Needs attention
            </CardTitle>
          </CardHeader>
          <CardContent>
            {inventory.error || !inventory.data ? (
              <p className="text-sm text-muted-foreground">Unable to load attention items.</p>
            ) : outOfStock.length === 0 ? (
              <p className="text-sm text-muted-foreground">Nothing needs attention right now.</p>
            ) : (
              <ul className="space-y-2">
                {outOfStock.map((item) => (
                  <li key={item.sku}>
                    <button
                      onClick={() => navigate('/inventory')}
                      className="w-full rounded-lg border border-border bg-muted/50 p-3 text-left transition-colors hover:border-amber-200 hover:bg-amber-50"
                    >
                      <p className="font-medium text-foreground">{item.sku}</p>
                      <p className="text-xs text-muted-foreground">{item.name} — out of stock</p>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  )
}
