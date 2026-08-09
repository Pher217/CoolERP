import { Card } from '@/components/ui/card'
import { cn } from '@/lib/utils'

type KpiCardProps = {
  label: string
  value: string
  subText?: string
  /** Semantic tint for the right-edge accent bar. */
  tone?: 'default' | 'positive' | 'negative'
}

const toneBar = {
  default: 'bg-primary',
  positive: 'bg-[#16a34a]',
  negative: 'bg-[#dc2626]',
} as const

export function KpiCard({ label, value, subText, tone = 'default' }: KpiCardProps) {
  return (
    // min-w-0 lets the card shrink inside a grid track; without it the long
    // currency string forces an overflow instead of scaling down.
    <Card className="relative min-w-0 overflow-hidden px-5 py-4">
      <p className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">
        {label}
      </p>
      {/* Money is the widest content here, so it gets tabular figures and a
          size that steps up only when the column is actually wide enough.
          title= keeps the full value reachable if it ever does truncate. */}
      <p
        title={value}
        className="mt-2 truncate text-2xl font-bold tabular-nums tracking-tight text-foreground xl:text-[1.75rem]"
      >
        {value}
      </p>
      {subText && <p className="mt-1 truncate text-xs text-muted-foreground">{subText}</p>}
      <span aria-hidden className={cn('absolute inset-y-0 right-0 w-1', toneBar[tone])} />
    </Card>
  )
}
