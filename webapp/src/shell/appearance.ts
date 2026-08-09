/** The validated workspace accent applied to the canonical token layer. */

export const DEFAULT_PRIMARY = '#2563EB'

/**
 * Keep malformed old data from becoming CSS input. The server applies the same
 * shape and contrast rule; this client check is a rendering boundary, not
 * authority or validation for a write.
 */
export function canonicalPrimary(value: string | undefined): string {
  if (value === undefined || !/^#[0-9A-Fa-f]{6}$/.test(value)) return DEFAULT_PRIMARY
  return value.toUpperCase()
}

/** Apply only action tokens. Brand, focus, status and canvas are independent. */
export function applyWorkspacePrimary(value: string | undefined): void {
  const primary = canonicalPrimary(value)
  const root = document.documentElement.style
  root.setProperty('--tf-primary', primary)
  root.setProperty('--tf-primary-hover', `color-mix(in srgb, ${primary}, #000 14%)`)
  root.setProperty('--tf-primary-pressed', `color-mix(in srgb, ${primary}, #000 24%)`)
}
