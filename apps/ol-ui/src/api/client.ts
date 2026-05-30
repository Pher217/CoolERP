/**
 * Thin fetch wrapper for the ol-api REST endpoints.
 *
 * Base URL is set via VITE_API_BASE env var (default: http://localhost:3000).
 * The gen:api script generates src/api/schema.d.ts from the live OpenAPI spec;
 * until then, types here are kept loose intentionally.
 */

const API_BASE = (import.meta.env.VITE_API_BASE as string | undefined) ?? 'http://localhost:3000'

export type HealthResponse = {
  status: 'ok' | string
}

export type BalanceResponse = {
  account_code: string
  /** Total debits in integer cents. */
  debits: number
  /** Total credits in integer cents. */
  credits: number
  /** Type-normalised signed balance in integer cents. */
  balance: number
}

class ApiError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
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

export const api = {
  /** GET /health */
  health(): Promise<HealthResponse> {
    return get<HealthResponse>('/health')
  },

  /** GET /accounts/{code}/balance */
  accountBalance(code: string | number): Promise<BalanceResponse> {
    return get<BalanceResponse>(`/accounts/${code}/balance`)
  },
}
