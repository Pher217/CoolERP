/**
 * Thin fetch wrapper for the ol-api REST endpoints.
 *
 * Base URL is set via VITE_API_BASE env var (default: http://localhost:3000).
 * The gen:api script generates src/api/schema.d.ts from the live OpenAPI spec;
 * until then, types here are hand-written to match the documented API shapes.
 */

const API_BASE = (import.meta.env.VITE_API_BASE as string | undefined) ?? 'http://localhost:3000'

export type HealthResponse = {
  status: 'ok' | string
}

export type BalanceResponse = {
  account_code: string
  /** ISO-4217 currency code (e.g. "EUR"). */
  currency: string
  /** Total debits in integer cents. */
  debits: number
  /** Total credits in integer cents. */
  credits: number
  /** Type-normalised signed balance in integer cents. */
  balance: number
}

export type JournalEntryLine = {
  account_code: string
  /** Integer cents. */
  debit: number
  /** Integer cents. */
  credit: number
}

export type PostJournalEntryBody = {
  idempotency_key: string
  journal_code: string
  entry_date: string // "YYYY-MM-DD"
  memo?: string
  reference?: string
  actor: string
  lines: JournalEntryLine[]
}

export type PostJournalEntryResponse = {
  entry_id: number
  balanced: boolean
  replayed: boolean
}

export type ApiErrorBody = {
  error: {
    code: string
    message: string
  }
}

export type ProcessListResponse = {
  processes: string[]
}

export type PostingRule = {
  debit: string
  credit: string
}

export type ProcessTransition = {
  from: string
  to: string
  capability?: string
  posting_rule?: PostingRule
  guards?: string[]
}

export type ProcessStep = {
  state: string
  description?: string
  fields: Array<{
    name: string
    label: string
    field_type: string
    required: boolean
  }>
  documents: string[]
  gates: string[]
  kpis: string[]
}

export type ProcessDetailResponse = {
  /** Canonical process name (preferred). */
  name: string
  /** Legacy alias kept for backwards compatibility. */
  process: string
  states: string[]
  transitions: ProcessTransition[]
  mermaid: string
  steps: ProcessStep[]
}

export type ChatTurn = {
  role: 'user' | 'assistant'
  content: string
}

export type ChatAction = {
  tool: string
  args: Record<string, unknown>
  result?: unknown
  error?: string
}

export type ChatView = {
  module: 'ledger' | 'inventory' | 'workflows' | 'processes'
  focus?: string | null
}

export type ChatResponse = {
  reply: string
  actions: ChatAction[]
  view?: ChatView | null
}

export type InventoryItem = {
  sku: string
  name: string
  on_hand: string
}

export type InventoryResponse = {
  items: InventoryItem[]
}

export type MetricsOverviewResponse = {
  /** Integer cents. */
  cash_cents: number
  /** Integer cents. */
  revenue_cents: number
  /** Integer cents. */
  expenses_cents: number
  /** Integer cents. */
  ar_cents: number
  /** Integer cents. */
  ap_cents: number
  account_count: number
}

export type ArAgingBucket = {
  /** Label e.g. "current", "1-30", "31-60", "61-90", "90+" */
  label: string
  /** Integer cents. */
  total_cents: number
}

export type ArAgingResponse = {
  buckets: ArAgingBucket[]
}

export class ApiError extends Error {
  status: number
  code?: string
  constructor(status: number, message: string, code?: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
  }
}

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    headers: { Accept: 'application/json' },
  })
  if (!res.ok) {
    throw new ApiError(res.status, `GET ${path} → ${res.status} ${res.statusText}`)
  }
  return res.json() as Promise<T>
}

async function post<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    method: 'POST',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(body),
  })
  if (!res.ok) {
    // Try to parse structured error envelope { error: { code, message } }
    let code: string | undefined
    let message = `POST ${path} → ${res.status} ${res.statusText}`
    try {
      const errBody = (await res.json()) as ApiErrorBody
      if (errBody?.error?.message) {
        message = errBody.error.message
        code = errBody.error.code
      }
    } catch {
      // body not parseable — keep generic message
    }
    throw new ApiError(res.status, message, code)
  }
  return res.json() as Promise<T>
}

export const api = {
  /** GET /health */
  health(): Promise<HealthResponse> {
    return get<HealthResponse>('/health')
  },

  /** GET /accounts/{code}/balance */
  accountBalance(code: string | number): Promise<BalanceResponse> {
    return get<BalanceResponse>(`/accounts/${code}/balance`)
  },

  /** GET /inventory */
  inventory(): Promise<InventoryResponse> {
    return get<InventoryResponse>('/inventory')
  },

  /** GET /processes */
  listProcesses(): Promise<ProcessListResponse> {
    return get<ProcessListResponse>('/processes')
  },

  /** GET /processes/{name} */
  getProcess(name: string): Promise<ProcessDetailResponse> {
    return get<ProcessDetailResponse>(`/processes/${name}`)
  },

  /**
   * POST /journal-entries
   * Generates idempotency_key automatically via crypto.randomUUID().
   */
  postJournalEntry(
    body: Omit<PostJournalEntryBody, 'idempotency_key'>,
  ): Promise<PostJournalEntryResponse> {
    const payload: PostJournalEntryBody = {
      ...body,
      idempotency_key: crypto.randomUUID(),
    }
    return post<PostJournalEntryResponse>('/journal-entries', payload)
  },

  /**
   * POST /chat
   * Send a message to the AI assistant with optional conversation history.
   * Returns the assistant reply and any ledger actions taken.
   */
  chat(message: string, history: ChatTurn[] = []): Promise<ChatResponse> {
    return post<ChatResponse>('/chat', { message, history })
  },

  /** GET /metrics/overview */
  metricsOverview(): Promise<MetricsOverviewResponse> {
    return get<MetricsOverviewResponse>('/metrics/overview')
  },

  /** GET /metrics/ar-aging */
  arAging(): Promise<ArAgingResponse> {
    return get<ArAgingResponse>('/metrics/ar-aging')
  },
}
