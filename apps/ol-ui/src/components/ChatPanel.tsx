import { useRef, useState } from 'react'
import { api, type ChatAction, type ChatTurn } from '../api/client.ts'
import { formatMoney } from '../lib/money.ts'

// ─── Action chip ──────────────────────────────────────────────────────────────

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
      className={`chat-action-chip ${isError ? 'chat-action-chip--error' : ''}`}
      title={isError ? action.error : JSON.stringify(action.result, null, 2)}
    >
      {label}
    </span>
  )
}

// ─── Message bubble ───────────────────────────────────────────────────────────

type Message =
  | { role: 'user'; content: string }
  | { role: 'assistant'; content: string; actions: ChatAction[] }

function MessageBubble({ msg }: { msg: Message }) {
  const isUser = msg.role === 'user'
  return (
    <div className={`chat-message ${isUser ? 'chat-message--user' : 'chat-message--assistant'}`}>
      <div className="chat-bubble">{msg.content}</div>
      {msg.role === 'assistant' && msg.actions.length > 0 && (
        <div className="chat-actions">
          {msg.actions.map((a, i) => (
            <ActionChip key={i} action={a} />
          ))}
        </div>
      )}
    </div>
  )
}

// ─── ChatPanel ────────────────────────────────────────────────────────────────

type ChatPanelProps = {
  onView?: (module: string, focus?: string | null) => void
}

export function ChatPanel({ onView }: ChatPanelProps) {
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState('')
  const [sending, setSending] = useState(false)
  const bottomRef = useRef<HTMLDivElement>(null)

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

    // Build history from prior turns (skip the message we just added).
    const history: ChatTurn[] = messages.map((m) => ({
      role: m.role,
      content: m.content,
    }))

    try {
      const resp = await api.chat(text, history)
      if (resp.view) {
        onView?.(resp.view.module, resp.view.focus)
      }
      const assistantMsg: Message = {
        role: 'assistant',
        content: resp.reply,
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
    <div className="widget chat-panel">
      <h2>AI Assistant</h2>

      <div className="chat-messages">
        {messages.length === 0 && (
          <p className="muted chat-empty">
            Ask about balances, or say "post €100 from 1000 to 4000 today".
          </p>
        )}
        {messages.map((msg, i) => (
          <MessageBubble key={i} msg={msg} />
        ))}
        {sending && (
          <div className="chat-message chat-message--assistant">
            <div className="chat-bubble chat-bubble--thinking">Thinking…</div>
          </div>
        )}
        <div ref={bottomRef} />
      </div>

      <div className="chat-input-row">
        <input
          className="chat-input"
          type="text"
          placeholder="Ask the assistant…"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          disabled={sending}
          aria-label="Chat message"
        />
        <button
          className="btn-primary"
          onClick={() => void send()}
          disabled={sending || !input.trim()}
        >
          {sending ? '…' : 'Send'}
        </button>
      </div>
    </div>
  )
}
