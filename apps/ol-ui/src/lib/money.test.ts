import { describe, it, expect } from 'vitest'
import { formatMoney } from './money.ts'

describe('formatMoney', () => {
  it('formats USD cents correctly', () => {
    expect(formatMoney(10050, 'USD', 'en-US')).toBe('$100.50')
  })

  it('formats zero cents', () => {
    expect(formatMoney(0, 'USD', 'en-US')).toBe('$0.00')
  })

  it('formats EUR in French locale', () => {
    const result = formatMoney(25099, 'EUR', 'fr-FR')
    expect(result).toContain('250')
    expect(result).toContain('99')
    expect(result).toContain('€')
  })

  it('formats a large negative amount', () => {
    expect(formatMoney(-500, 'USD', 'en-US')).toBe('-$5.00')
  })

  it('keeps the sign for small negative amounts (regression)', () => {
    // -5 cents = -$0.05 — must NOT render as positive $0.05.
    expect(formatMoney(-5, 'USD', 'en-US')).toBe('-$0.05')
  })

  it('keeps the sign for sub-unit negatives down to -1 cent', () => {
    expect(formatMoney(-1, 'USD', 'en-US')).toBe('-$0.01')
  })

  it('formats single-digit cents with leading zero', () => {
    expect(formatMoney(101, 'USD', 'en-US')).toBe('$1.01')
  })

  it('formats large amounts correctly', () => {
    expect(formatMoney(1234567, 'USD', 'en-US')).toBe('$12,345.67')
  })

  it('throws when cents is not an integer', () => {
    expect(() => formatMoney(10.5, 'USD', 'en-US')).toThrow('must be an integer')
  })

  it('formats CHF in Swiss German locale', () => {
    const result = formatMoney(99900, 'CHF', 'de-CH')
    expect(result).toContain('999')
    expect(result).toContain('00')
  })
})
