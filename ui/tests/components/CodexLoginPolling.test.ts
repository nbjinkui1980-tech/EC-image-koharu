import { describe, expect, it } from 'vitest'

import { codexLoginPollMs, codexLoginTimeoutMs } from '@/components/SettingsDialog'

describe('Codex login polling', () => {
  it('polls only while an open login is pending', () => {
    expect(codexLoginPollMs(true, 'pending', 3)).toBe(3000)
    expect(codexLoginPollMs(true, 'succeeded', 3)).toBeNull()
    expect(codexLoginPollMs(true, 'failed', 3)).toBeNull()
    expect(codexLoginPollMs(false, 'pending', 3)).toBeNull()
  })

  it('uses the server timeout only while an open login is pending', () => {
    expect(codexLoginTimeoutMs(true, 'pending', 120)).toBe(120000)
    expect(codexLoginTimeoutMs(false, 'pending', 120)).toBeNull()
    expect(codexLoginTimeoutMs(true, 'succeeded', 120)).toBeNull()
    expect(codexLoginTimeoutMs(true, 'failed', 120)).toBeNull()
  })
})
