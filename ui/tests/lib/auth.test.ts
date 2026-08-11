import { afterEach, describe, expect, it, vi } from 'vitest'

import { fetchWithAuth } from '@/lib/api/fetch'
import {
  bootstrapDesktopSession,
  exchangeSession,
  isAuthenticated,
  notifyAuthenticationRequired,
} from '@/lib/auth'
import { onAuthenticationRequired } from '@/lib/auth'

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

  it('notifies the bootstrap when a raw authenticated request receives 401', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(null, { status: 401 }))
    const listener = vi.fn()
    const unsubscribe = onAuthenticationRequired(listener)

    const response = await fetchWithAuth('/api/v1/raw', { method: 'PUT' })

    expect(response.status).toBe(401)
    expect(listener).toHaveBeenCalledTimes(1)
    unsubscribe()
  })

  it('requests the desktop bootstrap proof at most once per page lifetime', async () => {
    const invoke = vi.fn().mockResolvedValue('desktop-proof')
    vi.doMock('@tauri-apps/api/core', () => ({ invoke }))
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response(null, { status: 204 }))

    await bootstrapDesktopSession()

    expect(invoke).toHaveBeenCalledTimes(1)
    await expect(bootstrapDesktopSession()).rejects.toThrow('desktop restart required')
    expect(invoke).toHaveBeenCalledTimes(1)
  })
})
