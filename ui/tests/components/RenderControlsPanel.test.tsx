import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { http, HttpResponse } from 'msw'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { RenderControlsPanel } from '@/components/panels/RenderControlsPanel'
import * as sceneActions from '@/lib/io/scene'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'
import { usePreferencesStore } from '@/lib/stores/preferencesStore'
import { useSelectionStore } from '@/lib/stores/selectionStore'

import { renderWithQuery } from '../helpers'
import { server } from '../msw/server'

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

vi.mock('@/lib/io/scene', async () => {
  const actual = await vi.importActual<any>('@/lib/io/scene')
  return {
    ...actual,
    applyOp: vi.fn(),
    queueAutoRender: vi.fn(),
    runAutoRenderNow: vi.fn(),
  }
})

function sceneWithTextNodes(nodes: any[]) {
  const nodeMap: any = {}
  nodes.forEach((n) => {
    nodeMap[n.id] = {
      id: n.id,
      transform: { x: 0, y: 0, width: 10, height: 10, rotationDeg: 0 },
      visible: true,
      kind: { text: n.kind?.text ?? { style: { fontFamilies: ['Arial'] } } },
    }
  })
  return {
    epoch: 1,
    scene: {
      pages: {
        p1: { id: 'p1', name: 'P1', nodes: nodeMap },
      },
      project: { name: 'Proj' },
    },
  }
}

describe('RenderControlsPanel Font Assignment', () => {
  beforeEach(() => {
    useSelectionStore.getState().setPage('p1')
    useSelectionStore.getState().clear()
    usePreferencesStore.getState().setDefaultFont('Arial')
    useEditorUiStore.getState().clearError()
    vi.clearAllMocks()

    server.use(
      http.get('/api/v1/fonts', () =>
        HttpResponse.json([
          { familyName: 'Arial', postScriptName: 'Arial', source: 'system', cached: true },
          { familyName: 'Roboto', postScriptName: 'Roboto', source: 'system', cached: true },
          { familyName: 'Custom', postScriptName: 'Custom', source: 'system', cached: true },
        ]),
      ),
      http.get('/api/v1/google-fonts', () => HttpResponse.json({ fonts: [] })),
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(
          sceneWithTextNodes([
            { id: 't1', kind: { text: { style: { fontFamilies: ['Arial'] } } } },
            { id: 't2', kind: { text: { style: { fontFamilies: ['Arial'] } } } },
          ]),
        ),
      ),
    )
  })

  it('applying a font to a singular text box only updates that box', async () => {
    renderWithQuery(<RenderControlsPanel />)

    // Select node t1
    useSelectionStore.getState().select('t1', false)

    // Open font select
    const trigger = await screen.findByTestId('render-font-select')
    await userEvent.click(trigger)

    // Pick "Roboto"
    const option = await screen.findByText('Roboto')
    await userEvent.click(option)

    // Verify applyOp was called for t1
    await waitFor(() => expect(sceneActions.applyOp).toHaveBeenCalled())
    const lastOp = (sceneActions.applyOp as any).mock.calls[0][0]
    expect(lastOp).toHaveProperty('updateNode')
    expect(lastOp.updateNode.id).toBe('t1')
    expect(lastOp.updateNode.patch.data.text.style.fontFamilies).toEqual(['Roboto'])
    expect(usePreferencesStore.getState().defaultFont).toBe('Arial')
    expect(sceneActions.runAutoRenderNow).toHaveBeenCalledWith('p1')
  })

  it('bulk applying a font change (with selection) updates all selected boxes', async () => {
    renderWithQuery(<RenderControlsPanel />)

    // Select both nodes
    useSelectionStore.getState().selectMany(['t1', 't2'])

    // Open font select
    const trigger = await screen.findByTestId('render-font-select')
    await userEvent.click(trigger)

    // Pick "Roboto"
    const option = await screen.findByText('Roboto')
    await userEvent.click(option)

    // Verify applyOp was called with a batch
    await waitFor(() => expect(sceneActions.applyOp).toHaveBeenCalled())
    const lastOp = (sceneActions.applyOp as any).mock.calls[0][0]
    expect(lastOp).toHaveProperty('batch')
    expect(lastOp.batch.ops).toHaveLength(2)
  })

  it('changing global font updates every page node before rendering and preserves style', async () => {
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(
          sceneWithTextNodes([
            {
              id: 't1',
              kind: {
                text: {
                  style: {
                    fontFamilies: ['Arial'],
                    fontSize: 18,
                    color: [1, 2, 3, 255],
                    effect: { bold: true, italic: false },
                    stroke: { enabled: true, color: [7, 8, 9, 255], widthPx: 2 },
                    textAlign: 'left',
                  },
                },
              },
            },
            {
              id: 't2',
              kind: {
                text: {
                  style: {
                    fontFamilies: ['Arial'],
                    fontSize: 24,
                    color: [4, 5, 6, 255],
                    effect: { bold: false, italic: true },
                    stroke: { enabled: false, color: [10, 11, 12, 255], widthPx: 0 },
                    textAlign: 'right',
                  },
                },
              },
            },
          ]),
        ),
      ),
    )
    let finishApply: (() => void) | undefined
    vi.mocked(sceneActions.applyOp).mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finishApply = resolve
        }),
    )
    renderWithQuery(<RenderControlsPanel />)

    const trigger = await screen.findByTestId('render-font-select')
    await userEvent.click(trigger)
    const option = await screen.findByText('Custom')
    await userEvent.click(option)

    await waitFor(() => expect(sceneActions.applyOp).toHaveBeenCalled())
    expect(sceneActions.runAutoRenderNow).not.toHaveBeenCalled()
    expect(usePreferencesStore.getState().defaultFont).toBe('Arial')
    finishApply?.()
    await waitFor(() => expect(sceneActions.runAutoRenderNow).toHaveBeenCalledWith('p1'))

    expect(usePreferencesStore.getState().defaultFont).toBe('Custom')
    const op = vi.mocked(sceneActions.applyOp).mock.calls[0][0]
    expect(op).toHaveProperty('batch')
    expect(op.batch.ops).toHaveLength(2)
    expect(op.batch.ops[0].updateNode.patch.data.text.style).toMatchObject({
      fontFamilies: ['Custom'],
      fontSize: 18,
      color: [1, 2, 3, 255],
      effect: { bold: true, italic: false },
      stroke: { enabled: true, color: [7, 8, 9, 255], widthPx: 2 },
      textAlign: 'left',
    })
    expect(op.batch.ops[1].updateNode.patch.data.text.style).toMatchObject({
      fontFamilies: ['Custom'],
      fontSize: 24,
      color: [4, 5, 6, 255],
      effect: { bold: false, italic: true },
      stroke: { enabled: false, color: [10, 11, 12, 255], widthPx: 0 },
      textAlign: 'right',
    })
  })

  it('applies a font variant to the same global scope and renders immediately', async () => {
    usePreferencesStore.getState().setDefaultFont('Roboto')
    server.use(
      http.get('/api/v1/fonts', () =>
        HttpResponse.json([
          { familyName: 'Roboto', postScriptName: 'Roboto', source: 'system', cached: true },
          {
            familyName: 'Roboto',
            postScriptName: 'Roboto-Bold',
            source: 'system',
            cached: true,
          },
        ]),
      ),
    )

    renderWithQuery(<RenderControlsPanel />)

    const variantTrigger = await screen.findByRole('combobox')
    await userEvent.click(variantTrigger)
    await userEvent.click(await screen.findByText('Bold'))

    await waitFor(() => expect(sceneActions.runAutoRenderNow).toHaveBeenCalledWith('p1'))
    expect(usePreferencesStore.getState().defaultFont).toBe('Roboto-Bold')
    const op = vi.mocked(sceneActions.applyOp).mock.calls[0][0]
    expect(op.batch.ops).toHaveLength(2)
    for (const child of op.batch.ops) {
      expect(child.updateNode.patch.data.text.style.fontFamilies).toEqual(['Roboto-Bold'])
    }
  })

  it('stops before applying or rendering when a Google font download fails', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
    server.use(
      http.get('/api/v1/fonts', () =>
        HttpResponse.json([
          { familyName: 'Arial', postScriptName: 'Arial', source: 'system', cached: true },
          {
            familyName: 'Cloud',
            postScriptName: 'Cloud:400',
            source: 'google',
            cached: false,
          },
        ]),
      ),
      http.post('/api/v1/google-fonts/:family/fetch', () =>
        HttpResponse.json({ message: 'font download failed' }, { status: 500 }),
      ),
    )

    renderWithQuery(<RenderControlsPanel />)

    await userEvent.click(await screen.findByTestId('render-font-select'))
    await userEvent.click(await screen.findByText('Cloud'))

    await waitFor(() =>
      expect(useEditorUiStore.getState().error?.message).toContain('font download failed'),
    )
    expect(usePreferencesStore.getState().defaultFont).toBe('Arial')
    expect(sceneActions.applyOp).not.toHaveBeenCalled()
    expect(sceneActions.runAutoRenderNow).not.toHaveBeenCalled()
  })

  it('downloads a Google font before applying and rendering it', async () => {
    const events: string[] = []
    server.use(
      http.get('/api/v1/fonts', () =>
        HttpResponse.json([
          { familyName: 'Arial', postScriptName: 'Arial', source: 'system', cached: true },
          {
            familyName: 'Cloud',
            postScriptName: 'Cloud:400',
            source: 'google',
            cached: false,
          },
        ]),
      ),
      http.post('/api/v1/google-fonts/:family/fetch', () => {
        events.push('download')
        return HttpResponse.json({})
      }),
    )
    vi.mocked(sceneActions.applyOp).mockImplementationOnce(async () => {
      events.push('apply')
    })
    vi.mocked(sceneActions.runAutoRenderNow).mockImplementationOnce(async () => {
      expect(usePreferencesStore.getState().defaultFont).toBe('Cloud:400')
      events.push('render')
    })

    renderWithQuery(<RenderControlsPanel />)

    await userEvent.click(await screen.findByTestId('render-font-select'))
    await userEvent.click(await screen.findByText('Cloud'))

    await waitFor(() => expect(events.at(-1)).toBe('render'))
    const applyIndex = events.indexOf('apply')
    expect(events).toContain('download')
    expect(events.slice(0, applyIndex).every((event) => event === 'download')).toBe(true)
    expect(events.slice(applyIndex)).toEqual(['apply', 'render'])
    expect(usePreferencesStore.getState().defaultFont).toBe('Cloud:400')
  })

  it('preserves the global font and skips rendering when the scene update fails', async () => {
    vi.mocked(sceneActions.applyOp).mockRejectedValueOnce(new Error('scene apply failed'))

    renderWithQuery(<RenderControlsPanel />)

    await userEvent.click(await screen.findByTestId('render-font-select'))
    await userEvent.click(await screen.findByText('Custom'))

    await waitFor(() =>
      expect(useEditorUiStore.getState().error?.message).toContain('scene apply failed'),
    )
    expect(usePreferencesStore.getState().defaultFont).toBe('Arial')
    expect(sceneActions.runAutoRenderNow).not.toHaveBeenCalled()
  })

  it('shows auto when a selected block has no manual font size override', async () => {
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(
          sceneWithTextNodes([
            {
              id: 't1',
              kind: {
                text: {
                  style: { fontFamilies: ['Arial'] },
                  fontPrediction: { fontSizePx: 66, strokeWidthPx: 0, textColor: [0, 0, 0] },
                  detectedFontSizePx: 30,
                },
              },
            },
          ]),
        ),
      ),
    )

    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t1', false)

    const input = (await screen.findByTestId('render-font-size')) as HTMLInputElement
    await waitFor(() => expect(input.value).toBe(''))
    expect(input).toHaveAttribute('placeholder', 'auto')
  })

  it('opening the font color picker commits effective black as an explicit color', async () => {
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(
          sceneWithTextNodes([
            {
              id: 't1',
              kind: {
                text: {
                  fontPrediction: { fontSizePx: 66, strokeWidthPx: 0, textColor: [0, 0, 0] },
                },
              },
            },
          ]),
        ),
      ),
    )

    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t1', false)

    const trigger = await screen.findByTestId('render-color-trigger')
    await userEvent.click(trigger)

    await waitFor(() => expect(sceneActions.applyOp).toHaveBeenCalled())
    const op = (sceneActions.applyOp as any).mock.calls[0][0]
    expect(op.updateNode.id).toBe('t1')
    expect(op.updateNode.patch.data.text.style.color).toEqual([0, 0, 0, 255])
  })
})
