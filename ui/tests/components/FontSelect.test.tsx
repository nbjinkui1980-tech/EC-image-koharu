import { renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useGoogleFontPreview } from '@/components/ui/font-select'

const api = vi.hoisted(() => ({
  fetchGoogleFont: vi.fn(),
  getGetGoogleFontFileUrl: vi.fn(() => '/cached-font.ttf'),
}))

vi.mock('@/lib/api', () => api)

describe('FontSelect', () => {
  beforeEach(() => {
    api.fetchGoogleFont.mockReset().mockResolvedValue(undefined)
    api.getGetGoogleFontFileUrl.mockClear()
  })

  it('does not persist an uncached Google font just to preview a visible row', async () => {
    renderHook(() => useGoogleFontPreview('Cloud:400', 'google', true, false))

    expect(api.fetchGoogleFont).not.toHaveBeenCalled()
    expect(api.getGetGoogleFontFileUrl).not.toHaveBeenCalled()
  })
})
