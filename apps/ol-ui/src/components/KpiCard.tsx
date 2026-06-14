import { Card } from '@/components/ui/card'

type KpiCardProps = {
  label: string
  value: string
  subText?: string
}

export function KpiCard({ label, value, subText }: KpiCardProps) {
  return (
    <Card className="px-5 py-4">
      <p className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">
        {label}
      </p>
      <p className="mt-2 text-3xl font-bold tracking-tight text-foreground">{value}</p>
      {subText && (
        <p className="mt-1 text-xs text-muted-foreground">{subText}</p>
      )}
    </Card>
  )
}
