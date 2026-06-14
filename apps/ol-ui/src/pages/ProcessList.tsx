import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { GitBranch } from 'lucide-react'

import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { humanize } from '../lib/humanize.ts'
import { api } from '../api/client.ts'

export function ProcessList() {
  const navigate = useNavigate()
  const [processes, setProcesses] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(false)

  useEffect(() => {
    let cancelled = false

    api
      .listProcesses()
      .then((response) => {
        if (cancelled) return
        setProcesses(response.processes)
        setLoading(false)
      })
      .catch(() => {
        if (cancelled) return
        setError(true)
        setLoading(false)
      })

    return () => {
      cancelled = true
    }
  }, [])

  return (
    <div className="mx-auto max-w-6xl space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight text-foreground">Processes</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          End-to-end business process maps
        </p>
      </div>

      {loading && (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {Array.from({ length: 6 }).map((_, index) => (
            <Card key={index} className="shadow-sm">
              <CardHeader>
                <Skeleton className="h-5 w-2/3" />
              </CardHeader>
              <CardContent>
                <Skeleton className="h-4 w-1/2" />
              </CardContent>
            </Card>
          ))}
        </div>
      )}

      {error && (
        <Card className="shadow-sm">
          <CardHeader>
            <CardTitle>Unable to load processes</CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-sm text-muted-foreground">
              The API is unreachable. Start ol-api and try again.
            </p>
          </CardContent>
        </Card>
      )}

      {!loading && !error && (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {processes.map((process) => (
            <Card
              key={process}
              className="group cursor-pointer shadow-sm transition-all hover:border-primary/30 hover:shadow-md"
              onClick={() => navigate(`/processes/${process}`)}
            >
              <CardHeader className="pb-3">
                <div className="flex items-start justify-between">
                  <CardTitle className="text-base font-semibold transition-colors group-hover:text-primary">
                    {humanize(process)}
                  </CardTitle>
                  <GitBranch className="h-4 w-4 text-muted-foreground transition-colors group-hover:text-primary" />
                </div>
              </CardHeader>
              <CardContent className="pt-0">
                <p className="text-sm text-muted-foreground">{process}</p>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  )
}
