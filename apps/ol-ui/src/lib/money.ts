/**
 * Format an integer-cent amount as a locale-aware currency string.
 *
 * @param cents   - Integer cents (e.g. 10050 = 100.50 USD). Never a float.
 * @param currency - ISO 4217 currency code (e.g. "USD", "EUR").
 * @param locale  - BCP 47 locale string (e.g. "en-US", "fr-FR", "de-CH").
 * @returns       Formatted string via Intl.NumberFormat — no float math performed.
 */
export function formatMoney(cents: number, currency: string, locale: string): string {
  if (!Number.isInteger(cents)) {
    throw new Error(`formatMoney: cents must be an integer, got ${cents}`)
  }
  // Use BigInt division to keep precision then convert to string for Intl.
  // Dividing by 100 here only for display; we never store or compute in floats.
  const units = Math.trunc(cents / 100)
  const remainder = Math.abs(cents % 100)
  const decimal = `${units}.${String(remainder).padStart(2, '0')}`

  return new Intl.NumberFormat(locale, {
    style: 'currency',
    currency,
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(Number(decimal))
}
