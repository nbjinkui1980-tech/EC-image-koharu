import { beforeEach, describe, expect, it, vi } from 'vitest'

import { openVerificationUrl } from '@/lib/backend'

describe('openVerificationUrl', () => {
  beforeEach(() => {
    vi.restoreAllMocks()
  })

  it('rejects non-https, credential-bearing, and unknown-host urls', async () => {
    const openSpy = vi.spyOn(window, 'open').mockImplementation(() => null)

    await expect(openVerificationUrl('http://auth.openai.com/device')).rejects.toThrow()
    await expect(openVerificationUrl('https://user@auth.openai.com/device')).rejects.toThrow()
    await expect(openVerificationUrl('https://evil.example.com/device')).rejects.toThrow()
    expect(openSpy).not.toHaveBeenCalled()
  })

  it('allows the fixed authentication host over https', async () => {
    const openSpy = vi.spyOn(window, 'open').mockImplementation(() => null)

    await openVerificationUrl('https://auth.openai.com/device?code=abc')

    expect(openSpy).toHaveBeenCalledWith(
      'https://auth.openai.com/device?code=abc',
      '_blank',
      'noopener,noreferrer',
    )
  })
})
