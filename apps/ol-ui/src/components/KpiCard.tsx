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
  /* Reserved for genuine problems (e.g. overdue), not for ordinary liabilities. */
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
      {/* A clipped money figure is a trust defect in an accounting product --
          "€507,00…" or a plausible-looking "€50,700" is worse than a value that
          wraps onto two lines. So: never truncate, wrap instead, and keep
          tabular figures so columns of numbers stay aligned. */}
      <p className="mt-2 text-2xl font-bold tabular-nums tracking-tight break-words text-foreground xl:text-[1.75rem]">
        {value}
      </p>
      {subText && <p className="mt-1 truncate text-xs text-muted-foreground">{subText}</p>}
      <span aria-hidden className={cn('absolute inset-y-0 right-0 w-1', toneBar[tone])} />
    </Card>
  )
}
