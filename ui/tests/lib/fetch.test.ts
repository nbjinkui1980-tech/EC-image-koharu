import { afterEach, describe, expect, it, vi } from 'vitest'

const notifyAuthenticationRequired = vi.hoisted(() => vi.fn())

vi.mock('@/lib/auth', () => ({ notifyAuthenticationRequired }))

import { fetchApi } from '@/lib/api/fetch'

afterEach(() => vi.restoreAllMocks())

describe('authenticated API transport', () => {
  it('uses same-origin cookies and notifies before throwing on 401', async () => {
    const fetchMock = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValue(new Response(null, { status: 401 }))

    await expect(fetchApi('/api/v1/meta')).rejects.toMatchObject({ status: 401 })

    expect(fetchMock).toHaveBeenCalledWith('/api/v1/meta', { credentials: 'same-origin' })
    expect(notifyAuthenticationRequired).toHaveBeenCalledTimes(1)
  })
})
