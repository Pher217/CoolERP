import { useRef, useState } from 'react'
import { Send } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { api, type ChatAction, type ChatTurn } from '../api/client.ts'
import { formatMoney } from '../lib/money.ts'

function actionLabel(action: ChatAction): string {
  if (action.error) return `${action.tool} — error`

  if (action.tool === 'get_account_balance' && action.result) {
    const r = action.result as { account_code?: string; balance?: number; currency?: string }
    if (r.balance !== undefined && r.currency) {
      return `${action.tool} → ${r.account_code}: ${formatMoney(r.balance, r.currency, 'en-US')}`
    }
  }

  if (action.tool === 'post_journal_entry' && action.result) {
    const r = action.result as { entry_id?: number }
    if (r.entry_id !== undefined) {
      return `${action.tool} → entry #${r.entry_id}`
    }
  }

  if (action.tool === 'list_processes') {
    return 'list_processes → ok'
  }

  return action.tool
}

function ActionChip({ action }: { action: ChatAction }) {
  const label = actionLabel(action)
  const isError = Boolean(action.error)
  return (
    <span
      className={`inline-flex items-center rounded-full border px-2 py-0.5 text-[11px] font-medium ${
        isError
          ? 'border-red-200 bg-red-50 text-red-700'
          : 'border-blue-200 bg-blue-50 text-blue-700'
      }`}
      title={isError ? action.error : JSON.stringify(action.result, null, 2)}
    >
      {label}
    </span>
  )
}

type Message =
  | { role: 'user'; content: string }
  | { role: 'assistant'; content: string; actions: ChatAction[] }

function MessageBubble({ msg }: { msg: Message }) {
  const isUser = msg.role === 'user'
  return (
    <div className={`flex flex-col gap-1 ${isUser ? 'items-end' : 'items-start'}`}>
      <div
        className={`max-w-[85%] rounded-2xl px-4 py-2.5 text-sm leading-relaxed ${
          isUser
            ? 'rounded-br-sm bg-primary text-primary-foreground'
            : 'rounded-bl-sm bg-muted text-foreground'
        }`}
      >
        {msg.content}
      </div>
      {msg.role === 'assistant' && msg.actions.length > 0 && (
        <div className="flex flex-wrap gap-1 px-1">
          {msg.actions.map((a, i) => (
            <ActionChip key={i} action={a} />
          ))}
        </div>
      )}
    </div>
  )
}

type ChatPanelProps = {
  onView?: (module: string, focus?: string | null) => void
}

export function ChatPanel({ onView }: ChatPanelProps) {
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState('')
  const [sending, setSending] = useState(false)
  const bottomRef = useRef<HTMLDivElement>(null)
  // Stable for the life of this panel. The server derives tool idempotency keys
  // from it, so retrying an identical request deduplicates instead of posting
  // twice; without it the server mints a fresh id per request and cross-request
  // deduplication cannot engage at all.
  const conversationId = useRef<string>(crypto.randomUUID())

  function scrollToBottom() {
    setTimeout(() => bottomRef.current?.scrollIntoView({ behavior: 'smooth' }), 50)
  }

  async function send() {
    const text = input.trim()
    if (!text || sending) return

    const userMsg: Message = { role: 'user', content: text }
    const updatedMessages = [...messages, userMsg]
    setMessages(updatedMessages)
    setInput('')
    setSending(true)
    scrollToBottom()

    const history: ChatTurn[] = messages.map((m) => ({
      role: m.role,
      content: m.content,
    }))

    try {
      const resp = await api.chat(text, history, conversationId.current)
      if (resp.view) {
        onView?.(resp.view.module, resp.view.focus)
      }
      // A run that did not complete must never be rendered as if it had, and its
      // actions must survive: on a 502 the model call failed AFTER tools may
      // already have posted to the ledger, so `actions` is the user's only
      // record of what landed. Showing "could not reach the API" and dropping
      // them would hide real accounting work.
      const assistantMsg: Message = {
        role: 'assistant',
        content:
          resp.finish_reason === 'completed'
            ? resp.reply
            : `⚠️ ${resp.reply}`,
        actions: resp.actions,
      }
      setMessages([...updatedMessages, assistantMsg])
    } catch {
      const errMsg: Message = {
        role: 'assistant',
        content: 'Could not reach the API. Make sure ol-api is running.',
        actions: [],
      }
      setMessages([...updatedMessages, errMsg])
    } finally {
      setSending(false)
      scrollToBottom()
    }
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      void send()
    }
  }

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="flex-1 overflow-y-auto px-4 py-4">
        {messages.length === 0 && (
          <div className="flex flex-col items-center justify-center py-8 text-center">
            <div className="mb-3 flex h-10 w-10 items-center justify-center rounded-full bg-primary/10">
              <svg className="h-5 w-5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M8 10h.01M12 10h.01M16 10h.01M9 16H5a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v8a2 2 0 01-2 2h-5l-5 5v-5z" />
              </svg>
            </div>
            <p className="text-sm font-medium text-foreground">Ask the assistant</p>
            <p className="mt-1 text-xs text-muted-foreground">
              Try: "What's the cash balance?" or "Post €100 from 1000 to 4000 today"
            </p>
          </div>
        )}
        <div className="flex flex-col gap-3">
          {messages.map((msg, i) => (
            <MessageBubble key={i} msg={msg} />
          ))}
          {sending && (
            <div className="flex flex-col items-start gap-1">
              <div className="rounded-2xl rounded-bl-sm bg-muted px-4 py-2.5 text-sm italic text-muted-foreground">
                Thinking…
              </div>
            </div>
          )}
          <div ref={bottomRef} />
        </div>
      </div>

      <div className="flex-shrink-0 border-t bg-background/80 px-4 py-3 backdrop-blur">
        <div className="flex items-center gap-2">
          <Input
            className="flex-1 bg-background"
            type="text"
            placeholder="Ask the assistant…"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            disabled={sending}
            aria-label="Chat message"
          />
          <Button
            size="icon"
            onClick={() => void send()}
            disabled={sending || !input.trim()}
            aria-label="Send message"
          >
            <Send className="h-4 w-4" />
          </Button>
        </div>
      </div>
    </div>
  )
}
