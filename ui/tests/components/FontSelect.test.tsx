import { render, renderHook, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { FontSelect, useGoogleFontPreview } from '@/components/ui/font-select'

const api = vi.hoisted(() => ({
  fetchGoogleFont: vi.fn(),
  getGetGoogleFontFileUrl: vi.fn(() => '/cached-font.ttf'),
}))

vi.mock('@tanstack/react-virtual', () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getTotalSize: () => count * 28,
    getVirtualItems: () =>
      Array.from({ length: count }, (_, i) => ({
        index: i,
        start: i * 28,
        end: (i + 1) * 28,
        size: 28,
        key: i,
      })),
    measure: vi.fn(),
  }),
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

describe('useGoogleFontPreview font face ownership', () => {
  let addSpy: any
  let deleteSpy: any

  beforeEach(() => {
    addSpy = vi.spyOn(document.fonts, 'add').mockImplementation(() => document.fonts)
    deleteSpy = vi.spyOn(document.fonts, 'delete').mockImplementation(() => true)
  })

  it('deletes the added font face on unmount', async () => {
    vi.spyOn(FontFace.prototype, 'load').mockImplementation(function (this: FontFace) {
      return Promise.resolve(this)
    })
    const { result, unmount } = renderHook(() =>
      useGoogleFontPreview('Fam:400', 'google', true, true),
    )
    await waitFor(() => expect(result.current).toBe('ready'))
    expect(addSpy).toHaveBeenCalledTimes(1)
    const face = addSpy.mock.calls[0]![0]
    unmount()
    expect(deleteSpy).toHaveBeenCalledTimes(1)
    expect(deleteSpy).toHaveBeenCalledWith(face)
  })

  it('never adds a face whose load resolves after cancel', async () => {
    let resolveStale!: (face: unknown) => void
    vi.spyOn(FontFace.prototype, 'load').mockImplementation(
      () =>
        new Promise<FontFace>((resolve) => {
          resolveStale = resolve as (face: unknown) => void
        }),
    )
    const { unmount } = renderHook(() => useGoogleFontPreview('Fam:400', 'google', true, true))
    unmount()
    resolveStale(null)
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(addSpy).not.toHaveBeenCalled()
  })
})

describe('FontSelect favorite button keyboard isolation', () => {
  it('Enter on the favorite button toggles favorite without selecting the font', async () => {
    const onToggleFavorite = vi.fn()
    const onChange = vi.fn()
    render(
      <FontSelect
        value='Alpha'
        options={[
          { familyName: 'Alpha', postScriptName: 'AlphaPS', source: 'system', cached: true },
        ]}
        favoriteFonts={[]}
        onToggleFavorite={onToggleFavorite}
        onChange={onChange}
        data-testid='font-select'
      />,
    )
    await userEvent.click(screen.getByTestId('font-select'))
    await screen.findByPlaceholderText('Search fonts…')
    const texts = await screen.findAllByText('Alpha')
    const row = texts.map((el) => el.closest('div[role="button"]')).find(Boolean)!
    const favoriteButton = row.querySelector('button')!
    favoriteButton.focus()
    await userEvent.keyboard('{Enter}')
    expect(onToggleFavorite).toHaveBeenCalledTimes(1)
    expect(onChange).not.toHaveBeenCalled()
  })
})
