/**
 * Format an integer-cent amount as a locale-aware currency string.
 *
 * @param cents    - Integer cents (e.g. 10050 = 100.50). Never a float.
 * @param currency - ISO 4217 currency code (e.g. "EUR", "USD", "CHF").
 * @param locale   - BCP 47 locale string (e.g. "en-US", "fr-FR", "de-CH").
 * @returns        Formatted string via Intl.NumberFormat.
 *
 * Integer cents in; the single `/100` is for display only. `Intl.NumberFormat`
 * handles the sign and minor-unit rounding, so small negatives keep their sign.
 * Assumes a 2-decimal minor unit, matching the v0.1 single-currency cents model.
 */
export function formatMoney(cents: number, currency: string, locale: string): string {
  if (!Number.isInteger(cents)) {
    throw new Error(`formatMoney: cents must be an integer, got ${cents}`)
  }
  return new Intl.NumberFormat(locale, {
    style: 'currency',
    currency,
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(cents / 100)
}
