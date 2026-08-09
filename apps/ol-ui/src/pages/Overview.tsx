import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import {
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'

import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import {
  api,
  type ArAgingResponse,
  type InventoryResponse,
  type MetricsOverviewResponse,
} from '../api/client.ts'
import { formatMoney } from '../lib/money.ts'
import { KpiCard } from '../components/KpiCard.tsx'

type LoadState<T> = {
  data: T | null
  error: boolean
}

function emDash(): string {
  return '—'
}

/** CSS custom-property chart palette tokens (defined in index.css / Tailwind theme). */
const CHART_COLORS = [
  'var(--chart-1)',
  'var(--chart-2)',
  'var(--chart-3)',
  'var(--chart-4)',
  'var(--chart-5)',
]

export function Overview() {
  const navigate = useNavigate()

  const [metrics, setMetrics] = useState<LoadState<MetricsOverviewResponse>>({
    data: null,
    error: false,
  })
  const [aging, setAging] = useState<LoadState<ArAgingResponse>>({ data: null, error: false })
  const [inventory, setInventory] = useState<LoadState<InventoryResponse>>({
    data: null,
    error: false,
  })

  useEffect(() => {
    let cancelled = false

    async function load() {
      // Metrics overview + AR aging in parallel
      const [metricsResult, agingResult] = await Promise.allSettled([
        api.metricsOverview(),
        api.arAging(),
      ])

      if (cancelled) return

      if (metricsResult.status === 'fulfilled') {
        setMetrics({ data: metricsResult.value, error: false })
      } else {
        setMetrics({ data: null, error: true })
      }

      if (agingResult.status === 'fulfilled') {
        setAging({ data: agingResult.value, error: false })
      } else {
        setAging({ data: null, error: true })
      }

      // Inventory separately — errors are independent
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

  // KPI values — graceful fallback to em-dash on error / loading
  const cashValue =
    metrics.error || !metrics.data
      ? emDash()
      : formatMoney(metrics.data.cash_cents, 'EUR', 'en-US')
  const revenueValue =
    metrics.error || !metrics.data
      ? emDash()
      : formatMoney(metrics.data.revenue_cents, 'EUR', 'en-US')
  const expensesValue =
    metrics.error || !metrics.data
      ? emDash()
      : formatMoney(metrics.data.expenses_cents, 'EUR', 'en-US')
  const arValue =
    metrics.error || !metrics.data
      ? emDash()
      : formatMoney(metrics.data.ar_cents, 'EUR', 'en-US')
  const apValue =
    metrics.error || !metrics.data
      ? emDash()
      : formatMoney(metrics.data.ap_cents, 'EUR', 'en-US')

  // Bar chart: balances from metrics endpoint (cents → EUR for display)
  const balancesData =
    metrics.data
      ? [
          { name: 'Cash', value: metrics.data.cash_cents / 100 },
          { name: 'Revenue', value: metrics.data.revenue_cents / 100 },
          { name: 'Expenses', value: metrics.data.expenses_cents / 100 },
          { name: 'AR', value: metrics.data.ar_cents / 100 },
          { name: 'AP', value: metrics.data.ap_cents / 100 },
        ]
      : []

  // AR aging donut data
  const agingData =
    aging.data
      ? aging.data.buckets.map((b) => ({
          name: b.label,
          value: b.total_cents / 100,
          total_cents: b.total_cents,
        }))
      : []

  const agingTotal = agingData.reduce((sum, d) => sum + d.total_cents, 0)

  // Inventory attention list
  const outOfStock =
    inventory.error || !inventory.data
      ? []
      : inventory.data.items.filter((item) => Number(item.on_hand) === 0)

  return (
    <div className="mx-auto max-w-6xl space-y-8 pb-12">
      {/* Page heading */}
      <div>
        <h1 className="text-2xl font-bold tracking-tight text-foreground">Overview</h1>
        <p className="mt-1 text-sm text-muted-foreground">Financial snapshot and key metrics</p>
      </div>

      {/* KPI row — 5 cards, clear vertical separation from content below */}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-5">
        <KpiCard label="Cash on hand" value={cashValue} subText="Liquid assets" tone="positive" />
        <KpiCard label="Revenue" value={revenueValue} subText="Income accounts" tone="positive" />
        <KpiCard label="Expenses" value={expensesValue} subText="Cost accounts" />
        <KpiCard label="AR outstanding" value={arValue} subText="Receivables" />
        <KpiCard label="AP due" value={apValue} subText="Payables" tone="negative" />
      </div>

      {/* Second row: balances bar chart | AR aging donut */}
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <Card className="flex flex-col">
          <CardHeader>
            <CardTitle className="text-sm font-semibold uppercase tracking-widest text-muted-foreground">
              Balances
            </CardTitle>
          </CardHeader>
          <CardContent className="pt-2">
            {/* Fixed pixel height so ResponsiveContainer always gets a resolved height */}
            <div style={{ height: 256 }}>
              <ResponsiveContainer width="100%" height="100%">
                <BarChart data={balancesData} margin={{ top: 8, right: 16, bottom: 8, left: 0 }}>
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

        <Card className="flex flex-col">
          <CardHeader>
            <CardTitle className="text-sm font-semibold uppercase tracking-widest text-muted-foreground">
              AR aging
            </CardTitle>
          </CardHeader>
          <CardContent className="pt-2">
            {aging.error ? (
              <p className="text-sm text-muted-foreground">Unable to load AR aging data.</p>
            ) : agingData.length === 0 ? (
              <p className="text-sm text-muted-foreground">No AR aging data available.</p>
            ) : agingTotal === 0 ? (
              <p className="text-sm text-muted-foreground">No receivables outstanding.</p>
            ) : (
              <>
                {/* Fixed-height wrapper so ResponsiveContainer has a resolved pixel height */}
                <div style={{ height: 220 }}>
                  <ResponsiveContainer width="100%" height="100%">
                    <PieChart>
                      <Pie
                        data={agingData}
                        dataKey="value"
                        nameKey="name"
                        cx="50%"
                        cy="50%"
                        innerRadius={60}
                        outerRadius={90}
                        paddingAngle={2}
                      >
                        {agingData.map((_entry, index) => (
                          <Cell
                            key={`cell-${index}`}
                            fill={CHART_COLORS[index % CHART_COLORS.length]}
                          />
                        ))}
                      </Pie>
                      <Tooltip
                        contentStyle={{
                          background: 'var(--card)',
                          border: '1px solid var(--border)',
                          borderRadius: '0.5rem',
                          fontSize: '12px',
                          color: 'var(--foreground)',
                        }}
                        formatter={(value, name) => [
                          new Intl.NumberFormat('en-US', {
                            style: 'currency',
                            currency: 'EUR',
                          }).format(typeof value === 'number' ? value : 0),
                          name,
                        ]}
                      />
                    </PieChart>
                  </ResponsiveContainer>
                </div>

                {/* Legend — outside the chart height wrapper */}
                <ul className="mt-3 flex flex-wrap gap-x-4 gap-y-1">
                  {agingData.map((entry, index) => (
                    <li key={entry.name} className="flex items-center gap-1.5 text-xs text-muted-foreground">
                      <span
                        className="inline-block h-2.5 w-2.5 rounded-full"
                        style={{ background: CHART_COLORS[index % CHART_COLORS.length] }}
                      />
                      {entry.name}
                    </li>
                  ))}
                </ul>
              </>
            )}
          </CardContent>
        </Card>
      </div>

      {/* Third row: needs-attention | recent activity placeholder */}
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
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

        <Card>
          <CardHeader>
            <CardTitle className="text-sm font-semibold uppercase tracking-widest text-muted-foreground">
              Recent activity
            </CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-sm text-muted-foreground">Journal entries will appear here.</p>
          </CardContent>
        </Card>
      </div>
    </div>
  )
}
