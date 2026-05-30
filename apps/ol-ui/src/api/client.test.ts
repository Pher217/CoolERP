import { describe, it, expect } from 'vitest'
import { ApiError } from './client.ts'

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
