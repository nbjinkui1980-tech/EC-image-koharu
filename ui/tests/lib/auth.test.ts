import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { exchangeSession, isAuthenticated, notifyAuthenticationRequired } from '@/lib/auth'

afterEach(() => {
  notifyAuthenticationRequired()
  vi.restoreAllMocks()
})

describe('session authentication', () => {
  it('exchanges a bearer proof for a same-origin session', async () => {
    const fetchMock = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValue(new Response(null, { status: 204 }))

    await exchangeSession('proof')

    expect(fetchMock).toHaveBeenCalledWith('/api/v1/auth/session', {
      method: 'POST',
      headers: { Authorization: 'Bearer proof' },
      credentials: 'same-origin',
    })
    expect(isAuthenticated()).toBe(true)
  })

  it('stays unauthenticated when the exchange is rejected', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(null, { status: 401 }))

    await expect(exchangeSession('wrong')).rejects.toThrow('auth exchange failed: 401')
    expect(isAuthenticated()).toBe(false)
  })
})

describe('desktop bootstrap session', () => {
  beforeEach(() => {
    vi.resetModules()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('rejects and stays unauthenticated when the desktop IPC invoke call fails', async () => {
    const invoke = vi.fn().mockRejectedValue(new Error('IPC failed'))
    vi.doMock('@tauri-apps/api/core', () => ({ invoke }))
    const { bootstrapDesktopSession, isAuthenticated } = await import('@/lib/auth')

    await expect(bootstrapDesktopSession()).rejects.toThrow('IPC failed')
    expect(isAuthenticated()).toBe(false)
    expect(invoke).toHaveBeenCalledTimes(1)
  })

  it('rejects and stays unauthenticated when the session exchange fails after a successful invoke', async () => {
    const invoke = vi.fn().mockResolvedValue('desktop-proof')
    vi.doMock('@tauri-apps/api/core', () => ({ invoke }))
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(null, { status: 500 }))
    const { bootstrapDesktopSession, isAuthenticated } = await import('@/lib/auth')

    await expect(bootstrapDesktopSession()).rejects.toThrow('auth exchange failed: 500')
    expect(isAuthenticated()).toBe(false)
    expect(invoke).toHaveBeenCalledTimes(1)
  })

  it('requests the desktop bootstrap proof at most once per page lifetime', async () => {
    const invoke = vi.fn().mockResolvedValue('desktop-proof')
    vi.doMock('@tauri-apps/api/core', () => ({ invoke }))
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(null, { status: 204 }))
    const { bootstrapDesktopSession } = await import('@/lib/auth')

    await bootstrapDesktopSession()

    expect(invoke).toHaveBeenCalledTimes(1)
    await expect(bootstrapDesktopSession()).rejects.toThrow('desktop restart required')
    expect(invoke).toHaveBeenCalledTimes(1)
  })
})
