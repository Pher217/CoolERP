import { describe, expect, it } from 'vitest'
import { humanize } from './humanize.ts'

describe('humanize', () => {
  it('capitalises each underscore-separated word and joins with spaces', () => {
    expect(humanize('order_to_cash')).toBe('Order To Cash')
  })

  it('returns a single word capitalised', () => {
    expect(humanize('draft')).toBe('Draft')
  })

  it('handles empty string', () => {
    expect(humanize('')).toBe('')
  })
})
