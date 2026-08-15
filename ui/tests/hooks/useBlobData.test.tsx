import { act, renderHook } from '@testing-library/react'
import { http, HttpResponse } from 'msw'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useBlobImage } from '@/hooks/useBlobData'
import { queryClient } from '@/lib/queryClient'

import { withQueryClient } from '../helpers'
import { server } from '../msw/server'

let urlSeq = 0
let createSpy: ReturnType<typeof vi.fn>
let revokeSpy: ReturnType<typeof vi.fn>

class FakeImage {
  onload: (() => void) | null = null
  onerror: ((error: unknown) => void) | null = null
  set src(_value: string) {
    queueMicrotask(() => this.onload?.())
  }
}

function useBlobHandlers() {
  server.use(
    http.get('/api/v1/blobs/hash-1', () =>
      HttpResponse.arrayBuffer(new TextEncoder().encode('png-one').buffer),
    ),
    http.get('/api/v1/blobs/hash-2', () =>
      HttpResponse.arrayBuffer(new TextEncoder().encode('png-two').buffer),
    ),
  )
}

describe('useBlobImage', () => {
  beforeEach(() => {
    urlSeq = 0
    createSpy = vi.fn(() => `blob:url-${++urlSeq}`)
    revokeSpy = vi.fn()
    Object.defineProperty(URL, 'createObjectURL', { value: createSpy, configurable: true })
    Object.defineProperty(URL, 'revokeObjectURL', { value: revokeSpy, configurable: true })
    vi.stubGlobal('Image', FakeImage)
    useBlobHandlers()
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('caches the decoded blob, not an object URL', async () => {
    const { result } = renderHook(() => useBlobImage('hash-1'), {
      wrapper: withQueryClient(queryClient),
    })
    await vi.waitFor(() => expect(result.current.data).toBeTruthy())

    const cached = queryClient.getQueryData(['blobImage', 'hash-1'])
    expect(cached).toBeInstanceOf(Blob)
  })

  it('revokes component-owned urls on hash change and unmount', async () => {
    vi.useFakeTimers()
    try {
      const { result, rerender, unmount } = renderHook(({ hash }) => useBlobImage(hash), {
        initialProps: { hash: 'hash-1' as string },
        wrapper: withQueryClient(queryClient),
      })
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10)
      })
      const firstUrl = result.current.data
      expect(firstUrl).toBeTruthy()

      rerender({ hash: 'hash-2' })
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10)
      })
      expect(result.current.data).toBeTruthy()
      expect(result.current.data).not.toBe(firstUrl)

      await act(async () => {
        await vi.advanceTimersByTimeAsync(31_000)
      })
      expect(revokeSpy.mock.calls.map((call) => call[0])).toContain(firstUrl)

      const secondUrl = result.current.data
      unmount()
      await act(async () => {
        await vi.advanceTimersByTimeAsync(31_000)
      })
      expect(revokeSpy.mock.calls.map((call) => call[0])).toContain(secondUrl)
    } finally {
      vi.useRealTimers()
    }
  })
})
