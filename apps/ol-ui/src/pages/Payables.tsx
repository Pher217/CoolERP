import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'

export function Payables() {
  return (
    <div className="mx-auto max-w-6xl space-y-6">
      <h1 className="text-2xl font-bold tracking-tight">Payables</h1>
      <Card className="shadow-sm">
        <CardHeader>
          <CardTitle>Coming soon</CardTitle>
          <CardDescription>
            Vendor bills, payment runs, and liability tracking are on the roadmap.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">
            For now, journal entries can record payable movements via the ledger.
          </p>
        </CardContent>
      </Card>
    </div>
  )
}
