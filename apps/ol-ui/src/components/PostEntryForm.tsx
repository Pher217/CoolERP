import { useState } from 'react'
import { Plus, Trash2 } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { api, type JournalEntryLine, ApiError } from '../api/client.ts'

// shadcn label is not installed by default; we add a minimal one inline below.

function InlineLabel({ children, htmlFor }: { children: React.ReactNode; htmlFor?: string }) {
  return (
    <label
      htmlFor={htmlFor}
      className="text-xs font-semibold uppercase tracking-wide text-muted-foreground"
    >
      {children}
    </label>
  )
}

type Line = {
  account_code: string
  debit: string // raw input string (integer cents)
  credit: string // raw input string (integer cents)
}

type SubmitState =
  | { status: 'idle' }
  | { status: 'submitting' }
  | { status: 'success'; entry_id: number; replayed: boolean }
  | { status: 'error'; message: string; code?: string }

function emptyLine(): Line {
  return { account_code: '', debit: '0', credit: '0' }
}

function today(): string {
  return new Date().toISOString().slice(0, 10)
}

type Props = {
  onSuccess?: () => void
}

export function PostEntryForm({ onSuccess }: Props) {
  const [journal, setJournal] = useState('GEN')
  const [entryDate, setEntryDate] = useState(today())
  const [memo, setMemo] = useState('')
  const [actor, setActor] = useState('ui')
  const [lines, setLines] = useState<Line[]>([emptyLine(), emptyLine()])
  const [submitState, setSubmitState] = useState<SubmitState>({ status: 'idle' })

  function updateLine(index: number, field: keyof Line, value: string) {
    setLines((prev) => prev.map((l, i) => (i === index ? { ...l, [field]: value } : l)))
  }

  function addLine() {
    setLines((prev) => [...prev, emptyLine()])
  }

  function removeLine(index: number) {
    setLines((prev) => prev.filter((_, i) => i !== index))
  }

  function parseCents(raw: string): number {
    const n = parseInt(raw, 10)
    return isNaN(n) ? 0 : n
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    setSubmitState({ status: 'submitting' })

    const apiLines: JournalEntryLine[] = lines.map((l) => ({
      account_code: l.account_code.trim(),
      debit: parseCents(l.debit),
      credit: parseCents(l.credit),
    }))

    try {
      const result = await api.postJournalEntry({
        journal_code: journal.trim(),
        entry_date: entryDate,
        memo: memo.trim() || undefined,
        actor: actor.trim(),
        lines: apiLines,
      })
      setSubmitState({ status: 'success', entry_id: result.entry_id, replayed: result.replayed })
      onSuccess?.()
    } catch (err) {
      if (err instanceof ApiError) {
        setSubmitState({ status: 'error', message: err.message, code: err.code })
      } else {
        setSubmitState({ status: 'error', message: String(err) })
      }
    }
  }

  const isSubmitting = submitState.status === 'submitting'

  return (
    <Card className="shadow-sm">
      <CardHeader>
        <CardTitle>Post Journal Entry</CardTitle>
        <CardDescription>Create a balanced journal entry. Amounts are integer cents.</CardDescription>
      </CardHeader>
      <CardContent>
        <form className="space-y-5" onSubmit={handleSubmit}>
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
            <div className="space-y-2">
              <InlineLabel htmlFor="journal">Journal code</InlineLabel>
              <Input
                id="journal"
                type="text"
                value={journal}
                onChange={(e) => setJournal(e.target.value)}
                required
              />
            </div>
            <div className="space-y-2">
              <InlineLabel htmlFor="entryDate">Entry date</InlineLabel>
              <Input
                id="entryDate"
                type="date"
                value={entryDate}
                onChange={(e) => setEntryDate(e.target.value)}
                required
              />
            </div>
            <div className="space-y-2">
              <InlineLabel htmlFor="memo">Memo</InlineLabel>
              <Input
                id="memo"
                type="text"
                value={memo}
                onChange={(e) => setMemo(e.target.value)}
                placeholder="optional"
              />
            </div>
            <div className="space-y-2">
              <InlineLabel htmlFor="actor">Actor</InlineLabel>
              <Input
                id="actor"
                type="text"
                value={actor}
                onChange={(e) => setActor(e.target.value)}
                required
              />
            </div>
          </div>

          <div className="space-y-3">
            <InlineLabel>Lines (integer cents)</InlineLabel>
            <div className="rounded-lg border">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Account code</TableHead>
                    <TableHead className="text-right">Debit (¢)</TableHead>
                    <TableHead className="text-right">Credit (¢)</TableHead>
                    <TableHead className="w-12"></TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {lines.map((line, i) => (
                    <TableRow key={i}>
                      <TableCell>
                        <Input
                          type="text"
                          value={line.account_code}
                          onChange={(e) => updateLine(i, 'account_code', e.target.value)}
                          placeholder="e.g. 1000"
                          required
                        />
                      </TableCell>
                      <TableCell>
                        <Input
                          type="number"
                          value={line.debit}
                          onChange={(e) => updateLine(i, 'debit', e.target.value)}
                          min="0"
                          step="1"
                          className="text-right"
                        />
                      </TableCell>
                      <TableCell>
                        <Input
                          type="number"
                          value={line.credit}
                          onChange={(e) => updateLine(i, 'credit', e.target.value)}
                          min="0"
                          step="1"
                          className="text-right"
                        />
                      </TableCell>
                      <TableCell>
                        {lines.length > 2 && (
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            onClick={() => removeLine(i)}
                            aria-label="Remove line"
                          >
                            <Trash2 className="h-4 w-4 text-muted-foreground" />
                          </Button>
                        )}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
            <Button type="button" variant="outline" size="sm" onClick={addLine}>
              <Plus className="mr-2 h-4 w-4" />
              Add line
            </Button>
          </div>

          <div className="flex items-center gap-4">
            <Button type="submit" disabled={isSubmitting}>
              {isSubmitting ? 'Posting…' : 'Post entry'}
            </Button>
          </div>

          {submitState.status === 'success' && (
            <div className="rounded-md border border-emerald-200 bg-emerald-50 px-4 py-3 text-sm text-emerald-800">
              Entry #{submitState.entry_id} posted successfully.
              {submitState.replayed && ' (idempotent replay)'}
            </div>
          )}
          {submitState.status === 'error' && (
            <div className="rounded-md border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-800">
              {submitState.code && <span className="mr-1 font-mono font-semibold">[{submitState.code}]</span>}
              {submitState.message}
            </div>
          )}
        </form>
      </CardContent>
    </Card>
  )
}
