import { describe, it, expect, vi, afterEach } from 'vitest'
import { api, ApiError } from './client.ts'

describe('ApiError', () => {
  it('carries status and message', () => {
    const err = new ApiError(422, 'unbalanced entry')
    expect(err.status).toBe(422)
    expect(err.message).toBe('unbalanced entry')
    expect(err.name).toBe('ApiError')
    expect(err instanceof Error).toBe(true)
  })

  it('carries optional error code', () => {
    const err = new ApiError(422, 'entry not balanced', 'UNBALANCED')
    expect(err.code).toBe('UNBALANCED')
  })

  it('code is undefined when not provided', () => {
    const err = new ApiError(500, 'internal')
    expect(err.code).toBeUndefined()
  })

  it('is instanceof Error and ApiError', () => {
    const err = new ApiError(400, 'bad request')
    expect(err instanceof ApiError).toBe(true)
    expect(err instanceof Error).toBe(true)
  })
})

describe('api.chat', () => {
  const makeChatResponse = (overrides: {
    finish_reason: 'completed' | 'model_error' | 'transport_error'
    actions?: { tool: string; args: Record<string, unknown>; result?: unknown; error?: string }[]
  }) => ({
    reply: 'reply text',
    actions: overrides.actions ?? [],
    finish_reason: overrides.finish_reason,
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('returns a parsed ChatResponse on 200', async () => {
    const body = makeChatResponse({ finish_reason: 'completed' })
    global.fetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(body), { status: 200, statusText: 'OK' }),
    )

    const resp = await api.chat('hello')
    expect(resp).toEqual(body)
    expect(global.fetch).toHaveBeenCalledWith(
      'http://localhost:3000/chat',
      expect.objectContaining({
        method: 'POST',
        body: expect.stringContaining('"message":"hello"'),
      }),
    )
  })

  it('returns actions on a 502 model_error instead of throwing them away', async () => {
    const postedAction = {
      tool: 'post_journal_entry',
      args: { from: '1000', to: '4000', amount_cents: 10000 },
      result: { entry_id: 7 },
    }
    const body = makeChatResponse({
      finish_reason: 'model_error',
      actions: [postedAction],
    })
    global.fetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(body), { status: 502, statusText: 'Bad Gateway' }),
    )

    const resp = await api.chat('hello')
    expect(resp.finish_reason).toBe('model_error')
    expect(resp.actions).toEqual([postedAction])
  })

  it('returns actions on a 503 transport_error instead of throwing them away', async () => {
    const postedAction = {
      tool: 'receive_stock',
      args: { sku: 'WIDGET-1', quantity: 10 },
      result: { stock_move_id: 3 },
    }
    const body = makeChatResponse({
      finish_reason: 'transport_error',
      actions: [postedAction],
    })
    global.fetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(body), { status: 503, statusText: 'Service Unavailable' }),
    )

    const resp = await api.chat('hello')
    expect(resp.finish_reason).toBe('transport_error')
    expect(resp.actions).toEqual([postedAction])
  })

  it('passes conversation_id when provided', async () => {
    const body = makeChatResponse({ finish_reason: 'completed' })
    global.fetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(body), { status: 200, statusText: 'OK' }),
    )

    await api.chat('hello', [], 'conv-123')
    const callBody = JSON.parse(global.fetch.mock.calls[0][1].body as string)
    expect(callBody.conversation_id).toBe('conv-123')
  })

  it('throws ApiError for a non-200/502/503 status with a structured body', async () => {
    global.fetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: { code: 'INTERNAL', message: 'server on fire' } }), {
        status: 500,
        statusText: 'Internal Server Error',
      }),
    )

    await expect(api.chat('hello')).rejects.toSatisfy((err: ApiError) => {
      expect(err).toBeInstanceOf(ApiError)
      expect(err.status).toBe(500)
      expect(err.message).toBe('server on fire')
      expect(err.code).toBe('INTERNAL')
      return true
    })
  })

  it('throws ApiError for a non-200/502/503 status with an unparseable body', async () => {
    global.fetch = vi.fn().mockResolvedValue(
      new Response('teapot', { status: 418, statusText: "I'm a teapot" }),
    )

    await expect(api.chat('hello')).rejects.toSatisfy((err: ApiError) => {
      expect(err).toBeInstanceOf(ApiError)
      expect(err.status).toBe(418)
      expect(err.message).toBe("POST /chat → 418 I'm a teapot")
      expect(err.code).toBeUndefined()
      return true
    })
  })
})
