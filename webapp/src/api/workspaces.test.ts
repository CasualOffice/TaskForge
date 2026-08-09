import { describe, expect, it } from 'vitest'

import { normalizeWorkspace } from './workspaces'

describe('workspace wire compatibility', () => {
  it('gives an older response a safe default appearance', () => {
    expect(
      normalizeWorkspace({
        id: 'workspace-1',
        name: 'Demo',
        slug: 'demo',
        created_at: '2026-08-09T00:00:00Z',
      }),
    ).toMatchObject({
      version: 0,
      appearance: { primary_color: '#2563EB' },
    })
  })

  it('preserves the current typed appearance contract', () => {
    expect(
      normalizeWorkspace({
        id: 'workspace-1',
        name: 'Demo',
        slug: 'demo',
        version: 7,
        appearance: { primary_color: '#0F766E' },
        created_at: '2026-08-09T00:00:00Z',
      }),
    ).toMatchObject({
      version: 7,
      appearance: { primary_color: '#0F766E' },
    })
  })
})
