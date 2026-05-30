import { useState } from 'react'
import { api, type JournalEntryLine, ApiError } from '../api/client.ts'

type Line = {
  account_code: string
  debit: string  // raw input string (integer cents)
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
    <div className="widget">
      <h2>Post Journal Entry</h2>
      <form className="entry-form" onSubmit={handleSubmit}>
        <div className="form-row">
          <label>Journal code</label>
          <input
            type="text"
            value={journal}
            onChange={(e) => setJournal(e.target.value)}
            required
          />
        </div>
        <div className="form-row">
          <label>Entry date</label>
          <input
            type="date"
            value={entryDate}
            onChange={(e) => setEntryDate(e.target.value)}
            required
          />
        </div>
        <div className="form-row">
          <label>Memo</label>
          <input
            type="text"
            value={memo}
            onChange={(e) => setMemo(e.target.value)}
            placeholder="optional"
          />
        </div>
        <div className="form-row">
          <label>Actor</label>
          <input
            type="text"
            value={actor}
            onChange={(e) => setActor(e.target.value)}
            required
          />
        </div>

        <div className="lines-section">
          <h3>Lines <span className="muted">(integer cents)</span></h3>
          <table className="lines-table">
            <thead>
              <tr>
                <th>Account code</th>
                <th>Debit (¢)</th>
                <th>Credit (¢)</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {lines.map((line, i) => (
                <tr key={i}>
                  <td>
                    <input
                      type="text"
                      value={line.account_code}
                      onChange={(e) => updateLine(i, 'account_code', e.target.value)}
                      placeholder="e.g. 1000"
                      required
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      value={line.debit}
                      onChange={(e) => updateLine(i, 'debit', e.target.value)}
                      min="0"
                      step="1"
                    />
                  </td>
                  <td>
                    <input
                      type="number"
                      value={line.credit}
                      onChange={(e) => updateLine(i, 'credit', e.target.value)}
                      min="0"
                      step="1"
                    />
                  </td>
                  <td>
                    {lines.length > 2 && (
                      <button
                        type="button"
                        className="btn-remove"
                        onClick={() => removeLine(i)}
                        aria-label="Remove line"
                      >
                        ×
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <button type="button" className="btn-secondary" onClick={addLine}>
            + Add line
          </button>
        </div>

        <div className="form-actions">
          <button type="submit" className="btn-primary" disabled={isSubmitting}>
            {isSubmitting ? 'Posting…' : 'Post entry'}
          </button>
        </div>

        {submitState.status === 'success' && (
          <div className="success-banner">
            Entry #{submitState.entry_id} posted successfully.
            {submitState.replayed && ' (idempotent replay)'}
          </div>
        )}
        {submitState.status === 'error' && (
          <div className="error-banner">
            {submitState.code && <span className="error-code">[{submitState.code}]</span>}{' '}
            {submitState.message}
          </div>
        )}
      </form>
    </div>
  )
}
