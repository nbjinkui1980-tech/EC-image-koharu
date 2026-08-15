import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { CanvasDrawingConfig } from '@/hooks/useCanvasDrawing'
import { useMaskDrawing } from '@/hooks/useMaskDrawing'
import type { Page } from '@/lib/api/schemas'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'

const mocks = vi.hoisted(() => ({
  config: undefined as CanvasDrawingConfig | undefined,
  getConfig: vi.fn(),
  invalidateScene: vi.fn(),
  putMask: vi.fn(),
}))

vi.mock('@/hooks/useCanvasDrawing', () => ({
  useCanvasDrawing: (_dims: unknown, _pointer: unknown, config: CanvasDrawingConfig) => {
    mocks.config = config
    return { canvasRef: { current: null }, bind: () => ({}) }
  },
}))

vi.mock('@/lib/api', () => ({ getConfig: mocks.getConfig, putMask: mocks.putMask }))
vi.mock('@/lib/io/scene', () => ({ invalidateScene: mocks.invalidateScene }))

describe('useMaskDrawing', () => {
  beforeEach(() => {
    mocks.config = undefined
    mocks.getConfig.mockReset().mockResolvedValue({ pipeline: { inpainter: 'lama-manga' } })
    mocks.invalidateScene.mockReset().mockResolvedValue(undefined)
    mocks.putMask.mockReset().mockResolvedValue({ updated: true })
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true }))
    useEditorUiStore.setState({
      mode: 'select',
      showSegmentationMask: false,
      showInpaintedImage: false,
    })
  })

  it('eraser_mode_keeps_segment_mask_visible_and_updates_only_segment_endpoint', async () => {
    act(() => useEditorUiStore.getState().setMode('eraser'))
    expect(useEditorUiStore.getState().showSegmentationMask).toBe(true)

    renderHook(() =>
      useMaskDrawing({
        mode: 'eraser',
        page: { id: 'page-1', width: 8, height: 8 } as Page,
        pointerToDocument: () => ({ x: 0, y: 0 }),
        showMask: true,
        enabled: true,
      }),
    )
    await act(async () => {
      await mocks.config?.onFinalizeFullCanvas?.(new Uint8Array([1]), {
        x: 1,
        y: 2,
        width: 3,
        height: 4,
      })
    })

    // AR13-T03 RED: the mask must go through the generated putMask API, not a
    // duplicated raw fetch; errors and scene invalidation stay as they were.
    expect(mocks.putMask).toHaveBeenCalledTimes(1)
    const [id, role, body, params] = mocks.putMask.mock.calls[0]!
    expect(id).toBe('page-1')
    expect(role).toBe('segment')
    expect(body).toBeInstanceOf(Blob)
    expect(params).toMatchObject({
      pipeline: 'lama-manga',
      x: 1,
      y: 2,
      width: 3,
      height: 4,
    })
    expect(vi.mocked(fetch)).not.toHaveBeenCalled()
    expect(mocks.invalidateScene).toHaveBeenCalled()
  })

  it('stale_bitmap_is_closed_and_not_drawn_after_page_switch', async () => {
    const staleBitmap = { close: vi.fn() }
    const freshBitmap = { close: vi.fn() }
    let resolveStale: (bitmap: typeof staleBitmap) => void = () => {}
    const createImageBitmapMock = vi
      .fn()
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveStale = resolve
          }),
      )
      .mockResolvedValueOnce(freshBitmap)
    vi.stubGlobal('createImageBitmap', createImageBitmapMock)

    const drawImage = vi.fn()
    const ctx = {
      save: vi.fn(),
      restore: vi.fn(),
      clearRect: vi.fn(),
      drawImage,
      fillStyle: '',
      fillRect: vi.fn(),
    }

    const segmentData = new Uint8Array([1, 2, 3])
    const { rerender } = renderHook(
      ({ page }) =>
        useMaskDrawing({
          mode: 'repairBrush',
          page,
          segmentData,
          pointerToDocument: () => ({ x: 0, y: 0 }),
          showMask: true,
          enabled: true,
        }),
      { initialProps: { page: { id: 'page-1', width: 8, height: 8 } as Page } },
    )

    act(() => {
      mocks.config?.onCanvasInit?.(ctx as unknown as CanvasRenderingContext2D, {
        width: 8,
        height: 8,
        key: 'page-1',
      })
    })

    rerender({ page: { id: 'page-2', width: 8, height: 8 } as Page })
    act(() => {
      mocks.config?.onCanvasInit?.(ctx as unknown as CanvasRenderingContext2D, {
        width: 8,
        height: 8,
        key: 'page-2',
      })
    })

    await act(async () => {
      resolveStale(staleBitmap)
      await Promise.resolve()
    })

    expect(drawImage).toHaveBeenCalledTimes(1)
    expect(drawImage).toHaveBeenCalledWith(freshBitmap, 0, 0, 8, 8)
    expect(staleBitmap.close).toHaveBeenCalledTimes(1)
    expect(freshBitmap.close).toHaveBeenCalledTimes(1)
  })
})
