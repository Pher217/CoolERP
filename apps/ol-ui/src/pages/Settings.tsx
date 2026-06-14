import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'

export function Settings() {
  return (
    <div className="mx-auto max-w-6xl space-y-6">
      <h1 className="text-2xl font-bold tracking-tight">Settings</h1>
      <Card className="shadow-sm">
        <CardHeader>
          <CardTitle>Coming soon</CardTitle>
          <CardDescription>
            Organisation settings, users, and integrations will be configured here.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">
            This page is a placeholder while the settings API is being defined.
          </p>
        </CardContent>
      </Card>
    </div>
  )
}
