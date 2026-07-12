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
}))

vi.mock('@/hooks/useCanvasDrawing', () => ({
  useCanvasDrawing: (_dims: unknown, _pointer: unknown, config: CanvasDrawingConfig) => {
    mocks.config = config
    return { canvasRef: { current: null }, bind: () => ({}) }
  },
}))

vi.mock('@/lib/api', () => ({ getConfig: mocks.getConfig }))
vi.mock('@/lib/io/scene', () => ({ invalidateScene: mocks.invalidateScene }))

describe('useMaskDrawing', () => {
  beforeEach(() => {
    mocks.config = undefined
    mocks.getConfig.mockReset().mockResolvedValue({ pipeline: { inpainter: 'lama-manga' } })
    mocks.invalidateScene.mockReset().mockResolvedValue(undefined)
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

    const fetchMock = vi.mocked(fetch)
    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(fetchMock.mock.calls[0]?.[0]).toContain('/api/v1/pages/page-1/masks/segment?')
    expect(fetchMock.mock.calls[0]?.[0]).not.toContain('brushInpaint')
    expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({ method: 'PUT' })
  })
})
