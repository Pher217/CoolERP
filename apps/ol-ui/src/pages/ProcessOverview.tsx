import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import {
  ArrowLeft,
  BookOpen,
  ChevronRight,
  FileText,
  Gauge,
  Lock,
} from 'lucide-react'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { cn } from '@/lib/utils'
import { MermaidDiagram } from '../components/MermaidDiagram.tsx'
import { humanize } from '../lib/humanize.ts'
import { api, type ProcessDetailResponse, type ProcessStep } from '../api/client.ts'


type LoadState =
  | { status: 'loading' }
  | { status: 'ok'; data: ProcessDetailResponse }
  | { status: 'error'; message: string }
  | { status: 'offline' }

function StepBar({
  steps,
  selected,
  onSelect,
}: {
  steps: ProcessStep[]
  selected: string
  onSelect: (state: string) => void
}) {
  return (
    <div className="flex w-full items-center gap-2 overflow-x-auto pb-2">
      {steps.map((step, index) => (
        <div key={step.state} className="flex shrink-0 items-center gap-2">
          <button
            type="button"
            onClick={() => onSelect(step.state)}
            className={cn(
              'rounded-full px-4 py-1.5 text-sm font-medium whitespace-nowrap transition-colors',
              selected === step.state
                ? 'bg-primary text-primary-foreground shadow-sm'
                : 'bg-muted text-muted-foreground hover:bg-muted/80 hover:text-foreground',
            )}
          >
            {humanize(step.state)}
          </button>
          {index < steps.length - 1 && (
            <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
          )}
        </div>
      ))}
    </div>
  )
}

function EmptyBlock({ children }: { children: ReactNode }) {
  return <p className="text-sm text-muted-foreground">{children}</p>
}

export function ProcessOverview() {
  const { name } = useParams()
  const navigate = useNavigate()
  const [detail, setDetail] = useState<LoadState>({ status: 'loading' })
  const [selectedState, setSelectedState] = useState<string | null>(null)

  useEffect(() => {
    if (!name) return
    const processName = name
    let cancelled = false

    async function fetchDetail() {
      setDetail({ status: 'loading' })
      setSelectedState(null)
      try {
        const data = await api.getProcess(processName)
        if (!cancelled) {
          setDetail({ status: 'ok', data })
          setSelectedState(data.steps[0]?.state ?? null)
        }
      } catch (err: unknown) {
        if (cancelled) return
        const msg = err instanceof Error ? err.message : String(err)
        if (msg.includes('Failed to fetch') || msg.includes('NetworkError')) {
          setDetail({ status: 'offline' })
        } else {
          setDetail({ status: 'error', message: msg })
        }
      }
    }

    void fetchDetail()
    return () => {
      cancelled = true
    }
  }, [name])

  const selectedStep = useMemo(() => {
    if (detail.status !== 'ok') return null
    return (
      detail.data.steps.find((step) => step.state === selectedState) ??
      detail.data.steps[0] ??
      null
    )
  }, [detail, selectedState])

  const outgoingRules = useMemo(() => {
    if (!selectedStep || detail.status !== 'ok') return []
    return detail.data.transitions.filter(
      (transition) =>
        transition.from === selectedStep.state && transition.posting_rule,
    )
  }, [selectedStep, detail])

  const displayName =
    detail.status === 'ok'
      ? humanize(detail.data.name || detail.data.process)
      : humanize(name ?? '')

  if (!name) {
    return (
      <div className="mx-auto max-w-6xl space-y-6">
        <Card className="shadow-sm">
          <CardHeader>
            <CardTitle>Process not found</CardTitle>
            <CardDescription>No process name was provided in the URL.</CardDescription>
          </CardHeader>
          <CardContent>
            <Button variant="outline" onClick={() => navigate('/processes')}>
              Back to processes
            </Button>
          </CardContent>
        </Card>
      </div>
    )
  }

  return (
    <div className="mx-auto max-w-6xl space-y-6">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-3">
          <Button
            variant="ghost"
            size="icon"
            onClick={() => navigate('/processes')}
            aria-label="Back to processes"
          >
            <ArrowLeft className="h-5 w-5" />
          </Button>
          <div>
            <h1 className="text-2xl font-bold tracking-tight text-foreground">
              {displayName}
            </h1>
            <p className="text-sm text-muted-foreground">{name}</p>
          </div>
        </div>
      </div>

      {detail.status === 'loading' && (
        <div className="space-y-6">
          <Skeleton className="h-10 w-full" />
          <Card className="shadow-sm">
            <CardHeader>
              <Skeleton className="h-6 w-1/3" />
              <Skeleton className="h-4 w-2/3" />
            </CardHeader>
            <CardContent className="space-y-4">
              <Skeleton className="h-32 w-full" />
            </CardContent>
          </Card>
        </div>
      )}

      {detail.status === 'offline' && (
        <Card className="shadow-sm">
          <CardHeader>
            <CardTitle>API offline</CardTitle>
            <CardDescription>Start ol-api to load process details.</CardDescription>
          </CardHeader>
        </Card>
      )}

      {detail.status === 'error' && (
        <Card className="shadow-sm">
          <CardHeader>
            <CardTitle>Error</CardTitle>
            <CardDescription>{detail.message}</CardDescription>
          </CardHeader>
          <CardContent>
            <Button variant="outline" onClick={() => navigate('/processes')}>
              Back to processes
            </Button>
          </CardContent>
        </Card>
      )}

      {detail.status === 'ok' && detail.data.steps.length === 0 && (
        <Card className="shadow-sm">
          <CardHeader>
            <CardTitle>No steps defined</CardTitle>
            <CardDescription>This process does not yet define any steps.</CardDescription>
          </CardHeader>
        </Card>
      )}

      {detail.status === 'ok' && detail.data.steps.length > 0 && (
        <>
          <StepBar
            steps={detail.data.steps}
            selected={selectedStep?.state ?? ''}
            onSelect={setSelectedState}
          />

          <Card className="shadow-sm">
            <CardHeader>
              <CardTitle>
                {selectedStep ? humanize(selectedStep.state) : 'Step'}
              </CardTitle>
              {selectedStep?.description && (
                <CardDescription>{selectedStep.description}</CardDescription>
              )}
            </CardHeader>
            <CardContent className="space-y-6">
              {selectedStep && (
                <>
                  <section>
                    <h3 className="mb-2 text-sm font-semibold uppercase tracking-wider text-muted-foreground">
                      Fields
                    </h3>
                    {selectedStep.fields.length === 0 ? (
                      <EmptyBlock>No fields for this step.</EmptyBlock>
                    ) : (
                      <Table>
                        <TableHeader>
                          <TableRow>
                            <TableHead>Label</TableHead>
                            <TableHead>Type</TableHead>
                            <TableHead>Required</TableHead>
                          </TableRow>
                        </TableHeader>
                        <TableBody>
                          {selectedStep.fields.map((field) => (
                            <TableRow key={field.name}>
                              <TableCell className="font-medium">{field.label}</TableCell>
                              <TableCell className="capitalize">{field.field_type}</TableCell>
                              <TableCell>
                                {field.required ? (
                                  <Badge variant="default">Yes</Badge>
                                ) : (
                                  <span className="text-muted-foreground">—</span>
                                )}
                              </TableCell>
                            </TableRow>
                          ))}
                        </TableBody>
                      </Table>
                    )}
                  </section>

                  <section>
                    <h3 className="mb-2 text-sm font-semibold uppercase tracking-wider text-muted-foreground">
                      Documents
                    </h3>
                    {selectedStep.documents.length === 0 ? (
                      <EmptyBlock>No documents for this step.</EmptyBlock>
                    ) : (
                      <div className="flex flex-wrap gap-2">
                        {selectedStep.documents.map((document) => (
                          <Badge
                            key={document}
                            variant="secondary"
                            className="gap-1"
                          >
                            <FileText className="h-3 w-3" />
                            {document}
                          </Badge>
                        ))}
                      </div>
                    )}
                  </section>

                  <section>
                    <h3 className="mb-2 text-sm font-semibold uppercase tracking-wider text-muted-foreground">
                      Gates
                    </h3>
                    {selectedStep.gates.length === 0 ? (
                      <EmptyBlock>No gates for this step.</EmptyBlock>
                    ) : (
                      <ul className="space-y-1">
                        {selectedStep.gates.map((gate, index) => (
                          <li
                            key={index}
                            className="flex items-center gap-2 text-sm text-foreground"
                          >
                            <Lock className="h-3.5 w-3.5 text-muted-foreground" />
                            {gate}
                          </li>
                        ))}
                      </ul>
                    )}
                  </section>

                  <section>
                    <h3 className="mb-2 text-sm font-semibold uppercase tracking-wider text-muted-foreground">
                      KPIs
                    </h3>
                    {selectedStep.kpis.length === 0 ? (
                      <EmptyBlock>No KPIs for this step.</EmptyBlock>
                    ) : (
                      <div className="flex flex-wrap gap-2">
                        {selectedStep.kpis.map((kpi) => (
                          <Badge
                            key={kpi}
                            variant="outline"
                            className="gap-1"
                          >
                            <Gauge className="h-3 w-3" />
                            {kpi}
                          </Badge>
                        ))}
                      </div>
                    )}
                  </section>

                  {outgoingRules.length > 0 && (
                    <section className="rounded-lg border bg-muted/40 p-4">
                      <div className="mb-2 flex items-center gap-2 text-sm font-semibold text-foreground">
                        <BookOpen className="h-4 w-4 text-primary" />
                        Accounting
                      </div>
                      <div className="space-y-1">
                        {outgoingRules.map((transition, index) =>
                          transition.posting_rule ? (
                            <p
                              key={index}
                              className="text-sm text-muted-foreground"
                            >
                              <span className="font-medium text-foreground">
                                {transition.posting_rule.debit}
                              </span>{' '}
                              →{' '}
                              <span className="font-medium text-foreground">
                                {transition.posting_rule.credit}
                              </span>
                              {transition.to && (
                                <span className="ml-2 text-xs">
                                  (to {humanize(transition.to)})
                                </span>
                              )}
                            </p>
                          ) : null,
                        )}
                      </div>
                    </section>
                  )}
                </>
              )}
            </CardContent>
          </Card>

          <Card className="shadow-sm">
            <CardHeader>
              <CardTitle>State machine</CardTitle>
              <CardDescription>
                Full flow and transitions for {displayName}
              </CardDescription>
            </CardHeader>
            <CardContent>
              <Tabs defaultValue="diagram">
                <TabsList variant="line">
                  <TabsTrigger value="diagram">Diagram</TabsTrigger>
                  <TabsTrigger value="transitions">Transitions</TabsTrigger>
                </TabsList>
                <TabsContent value="diagram" className="pt-4">
                  <MermaidDiagram chart={detail.data.mermaid} />
                </TabsContent>
                <TabsContent value="transitions" className="pt-4">
                  {detail.data.transitions.length === 0 ? (
                    <EmptyBlock>No transitions defined.</EmptyBlock>
                  ) : (
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>From</TableHead>
                          <TableHead>To</TableHead>
                          <TableHead>Capability</TableHead>
                          <TableHead>Guards</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {detail.data.transitions.map((transition, index) => (
                          <TableRow key={index}>
                            <TableCell className="font-medium">
                              {humanize(transition.from)}
                            </TableCell>
                            <TableCell>{humanize(transition.to)}</TableCell>
                            <TableCell>
                              {transition.capability ?? (
                                <span className="text-muted-foreground">—</span>
                              )}
                            </TableCell>
                            <TableCell>
                              {transition.guards && transition.guards.length > 0
                                ? transition.guards.join(', ')
                                : '—'}
                            </TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  )}
                </TabsContent>
              </Tabs>
            </CardContent>
          </Card>
        </>
      )}
    </div>
  )
}
