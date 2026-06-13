type KpiCardProps = {
  label: string
  value: string
  subText?: string
}

export function KpiCard({ label, value, subText }: KpiCardProps) {
  return (
    <div className="rounded-xl border border-slate-200 bg-white p-5 shadow-sm">
      <p className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</p>
      <p className="mt-2 text-2xl font-bold text-slate-900">{value}</p>
      {subText && (
        <p className="mt-1 text-sm text-slate-500">{subText}</p>
      )}
    </div>
  )
}
