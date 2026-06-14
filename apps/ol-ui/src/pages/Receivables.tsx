import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'

export function Receivables() {
  return (
    <div className="mx-auto max-w-6xl space-y-6">
      <h1 className="text-2xl font-bold tracking-tight">Receivables</h1>
      <Card className="shadow-sm">
        <CardHeader>
          <CardTitle>Coming soon</CardTitle>
          <CardDescription>
            Customer invoices, payments, and ageing reports are on the roadmap.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">
            For now, journal entries can record receivable movements via the ledger.
          </p>
        </CardContent>
      </Card>
    </div>
  )
}
