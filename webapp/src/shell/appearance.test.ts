import { afterEach, describe, expect, it } from 'vitest'

import { DEFAULT_PRIMARY, applyWorkspacePrimary, canonicalPrimary } from './appearance'

afterEach(() => {
  document.documentElement.removeAttribute('style')
})

describe('workspace appearance', () => {
  it('canonicalizes the typed API value and rejects inert malformed data', () => {
    expect(canonicalPrimary('#1d4ed8')).toBe('#1D4ED8')
    expect(canonicalPrimary('blue')).toBe(DEFAULT_PRIMARY)
    expect(canonicalPrimary(undefined)).toBe(DEFAULT_PRIMARY)
  })

  it('changes action tokens without changing brand, focus, semantic, or canvas roles', () => {
    applyWorkspacePrimary('#0f766e')
    const style = document.documentElement.style
    expect(style.getPropertyValue('--tf-primary')).toBe('#0F766E')
    expect(style.getPropertyValue('--tf-primary-hover')).toContain('#0F766E')
    expect(style.getPropertyValue('--tf-brand')).toBe('')
    expect(style.getPropertyValue('--tf-focus')).toBe('')
    expect(style.getPropertyValue('--tf-danger')).toBe('')
    expect(style.getPropertyValue('--tf-bg')).toBe('')
  })
})
