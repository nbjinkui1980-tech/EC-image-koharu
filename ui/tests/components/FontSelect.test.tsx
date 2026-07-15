import { render, renderHook, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { FontSelect, useGoogleFontPreview } from '@/components/ui/font-select'

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

  it('signals online browsing once when search changes from empty to non-empty', async () => {
    const onBrowseOnlineFonts = vi.fn()
    render(
      <FontSelect
        data-testid='font-select'
        value='Arial'
        options={[
          {
            familyName: 'Arial',
            postScriptName: 'Arial',
            source: 'system',
            cached: true,
          },
        ]}
        onChange={vi.fn()}
        onBrowseOnlineFonts={onBrowseOnlineFonts}
      />,
    )

    await userEvent.click(screen.getByTestId('font-select'))
    await userEvent.type(screen.getByPlaceholderText('Search fonts…'), 'Roboto')

    expect(onBrowseOnlineFonts).toHaveBeenCalledTimes(1)
  })

  it('signals online browsing when a Google category is selected', async () => {
    const onBrowseOnlineFonts = vi.fn()
    render(
      <FontSelect
        data-testid='font-select'
        value='Arial'
        options={[
          {
            familyName: 'Arial',
            postScriptName: 'Arial',
            source: 'system',
            cached: true,
          },
        ]}
        onChange={vi.fn()}
        onBrowseOnlineFonts={onBrowseOnlineFonts}
      />,
    )

    await userEvent.click(screen.getByTestId('font-select'))
    await userEvent.click(screen.getByRole('button', { name: 'Sans' }))

    expect(onBrowseOnlineFonts).toHaveBeenCalledTimes(1)
  })
})
