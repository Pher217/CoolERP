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
  const inventorySubText =
    totalOnHand === null ? undefined : `${totalOnHand} units on hand`

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
    <div className="space-y-6">
      <h1 className="text-2xl font-bold text-slate-900">Overview</h1>

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <KpiCard label="Cash on hand" value={cashValue} subText="Current balance" />
        <KpiCard label="Revenue" value={revenueValue} subText="Account 4000" />
        <KpiCard label="Expenses" value={expensesValue} subText="Account 5000" />
        <KpiCard label="Inventory SKUs" value={inventoryValue} subText={inventorySubText} />
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <div className="rounded-xl border border-slate-200 bg-white p-5 shadow-sm lg:col-span-2">
          <h2 className="mb-4 text-sm font-semibold uppercase tracking-wide text-slate-500">
            Balances
          </h2>
          <div className="h-72">
            <ResponsiveContainer width="100%" height="100%">
              <BarChart data={chartData} margin={{ top: 8, right: 16, bottom: 8, left: 0 }}>
                <CartesianGrid strokeDasharray="3 3" stroke="#e2e8f0" />
                <XAxis dataKey="name" tick={{ fill: '#64748b', fontSize: 12 }} />
                <YAxis tick={{ fill: '#64748b', fontSize: 12 }} />
                <Tooltip
                  formatter={(value) => [
                    new Intl.NumberFormat('en-US', {
                      style: 'currency',
                      currency: 'EUR',
                    }).format(typeof value === 'number' ? value : 0),
                    'Balance',
                  ]}
                />
                <Bar dataKey="value" fill="#4f46e5" radius={[4, 4, 0, 0]} />
              </BarChart>
            </ResponsiveContainer>
          </div>
        </div>

        <div className="rounded-xl border border-slate-200 bg-white p-5 shadow-sm">
          <h2 className="mb-4 text-sm font-semibold uppercase tracking-wide text-slate-500">
            Needs attention
          </h2>

          {inventory.error || !inventory.data ? (
            <p className="text-sm text-slate-400">Unable to load attention items.</p>
          ) : outOfStock.length === 0 ? (
            <p className="text-sm text-slate-500">Nothing needs attention right now. 🎉</p>
          ) : (
            <ul className="space-y-2">
              {outOfStock.map((item) => (
                <li key={item.sku}>
                  <button
                    onClick={() => navigate('/inventory')}
                    className="w-full rounded-lg border border-slate-100 bg-slate-50 p-3 text-left transition-colors hover:border-amber-200 hover:bg-amber-50"
                  >
                    <p className="font-medium text-slate-800">{item.sku}</p>
                    <p className="text-xs text-slate-500">{item.name} — out of stock</p>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  )
}
