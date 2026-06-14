import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'

export function Accounts() {
  return (
    <div className="mx-auto max-w-6xl space-y-6">
      <h1 className="text-2xl font-bold tracking-tight">Accounts</h1>
      <Card className="shadow-sm">
        <CardHeader>
          <CardTitle>Coming soon</CardTitle>
          <CardDescription>
            The chart of accounts browser and editor is under construction.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">
            Use the General Ledger → Journal Entries page to post and view balances today.
          </p>
        </CardContent>
      </Card>
    </div>
  )
}
