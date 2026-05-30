import { describe, it, expect } from 'vitest'
import { formatMoney } from './money.ts'

describe('formatMoney', () => {
  it('formats USD cents correctly', () => {
    // 10050 cents = $100.50
    expect(formatMoney(10050, 'USD', 'en-US')).toBe('$100.50')
  })

  it('formats zero cents', () => {
    expect(formatMoney(0, 'USD', 'en-US')).toBe('$0.00')
  })

  it('formats EUR in French locale', () => {
    // 25099 cents = 250.99 EUR in fr-FR format
    const result = formatMoney(25099, 'EUR', 'fr-FR')
    // Intl formats may use non-breaking spaces; just check it contains "250" and "99"
    expect(result).toContain('250')
    expect(result).toContain('99')
    expect(result).toContain('€')
  })

  it('formats negative amounts', () => {
    // -500 cents = -$5.00
    expect(formatMoney(-500, 'USD', 'en-US')).toBe('-$5.00')
  })

  it('formats single-digit cents with leading zero', () => {
    // 101 cents = $1.01
    expect(formatMoney(101, 'USD', 'en-US')).toBe('$1.01')
  })

  it('formats large amounts correctly', () => {
    // 1234567 cents = $12,345.67
    expect(formatMoney(1234567, 'USD', 'en-US')).toBe('$12,345.67')
  })

  it('throws when cents is not an integer', () => {
    expect(() => formatMoney(10.5, 'USD', 'en-US')).toThrow('must be an integer')
  })

  it('formats CHF in Swiss German locale', () => {
    // 99900 cents = 999.00 CHF
    const result = formatMoney(99900, 'CHF', 'de-CH')
    expect(result).toContain('999')
    expect(result).toContain('00')
  })
})
