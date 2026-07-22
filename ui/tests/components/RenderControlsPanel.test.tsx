import { focusManager, onlineManager, QueryClient } from '@tanstack/react-query'
import { fireEvent, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { delay, http, HttpResponse } from 'msw'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { RenderControlsPanel } from '@/components/panels/RenderControlsPanel'
import { getGetConfigQueryKey, getGetGoogleFontsCatalogQueryKey } from '@/lib/api'
import type { Op } from '@/lib/api/schemas'
import * as sceneActions from '@/lib/io/scene'
import { queryClient } from '@/lib/queryClient'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'
import { useJobsStore } from '@/lib/stores/jobsStore'
import { usePreferencesStore } from '@/lib/stores/preferencesStore'
import { useSelectionStore } from '@/lib/stores/selectionStore'
import enUsTranslation from '@/public/locales/en-US/translation.json'
import zhCnTranslation from '@/public/locales/zh-CN/translation.json'

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
    invalidateScene: vi.fn(async () => undefined),
    queueAutoRender: vi.fn(),
    runAutoRenderNow: vi.fn(),
  }
})

function sceneWithTextNodes(nodes: any[]) {
  const nodeMap: any = {}
  nodes.forEach((n) => {
    nodeMap[n.id] = {
      id: n.id,
      transform: n.transform ?? { x: 0, y: 0, width: 10, height: 10, rotationDeg: 0 },
      visible: true,
      kind: { text: n.kind?.text ?? { style: { fontFamilies: ['Arial'] } } },
    }
  })
  return {
    epoch: 1,
    scene: {
      pages: {
        p1: { id: 'p1', name: 'P1', width: 900, height: 900, nodes: nodeMap },
      },
      project: { name: 'Proj' },
    },
  }
}

function fullyStyledTextNodes() {
  return [
    {
      id: 't1',
      kind: {
        text: {
          style: {
            fontFamilies: ['Arial'],
            fontSize: 18,
            color: [1, 2, 3, 255],
            effect: { bold: true, italic: false },
            stroke: { enabled: true, color: [7, 8, 9, 255], widthPx: 1.5 },
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
            fontFamilies: ['Roboto'],
            fontSize: 24,
            color: [4, 5, 6, 255],
            effect: { bold: false, italic: true },
            stroke: { enabled: false, color: [10, 11, 12, 255], widthPx: 2.5 },
            textAlign: 'center',
          },
        },
      },
    },
    {
      id: 't3',
      kind: {
        text: {
          style: {
            fontFamilies: ['Custom'],
            fontSize: 30,
            color: [13, 14, 15, 255],
            effect: { bold: true, italic: true },
            stroke: { enabled: true, color: [16, 17, 18, 255], widthPx: 3.5 },
            textAlign: 'right',
          },
        },
      },
    },
  ]
}

function englishAutomaticGroupNodes(): any[] {
  return [
    {
      id: 't2',
      transform: { x: 120, y: 1, width: 100, height: 20, rotationDeg: 0 },
      kind: {
        text: {
          style: { fontFamilies: ['Arial'] },
          text: '中文',
          translation: 'middle',
          sourceDirection: 'horizontal',
          fontPrediction: { fontSizePx: 90 },
          detectedFontSizePx: 76.79797,
        },
      },
    },
    {
      id: 't3',
      transform: { x: 240, y: 0.5, width: 100, height: 20, rotationDeg: 0 },
      kind: {
        text: {
          style: { fontFamilies: ['Arial'] },
          text: '中文',
          translation: 'right',
          sourceDirection: 'horizontal',
          detectedFontSizePx: 37.19812,
        },
      },
    },
    {
      id: 't1',
      transform: { x: 0, y: 0, width: 100, height: 20, rotationDeg: 0 },
      kind: {
        text: {
          style: { fontFamilies: ['Arial'] },
          text: '中文',
          translation: 'left',
          sourceDirection: 'horizontal',
          detectedFontSizePx: 37.437256,
        },
      },
    },
  ]
}

function expectBatchStyleUpdate(update: Record<string, unknown>) {
  const sourceById = new Map(
    fullyStyledTextNodes().map((node) => [node.id, node.kind.text.style] as const),
  )
  const op = vi.mocked(sceneActions.applyOp).mock.calls[0][0] as any
  expect(op.batch.ops).toHaveLength(3)
  for (const child of op.batch.ops) {
    const id = child.updateNode.id as string
    expect(child.updateNode.patch.data.text.style).toEqual({
      ...sourceById.get(id),
      ...update,
    })
  }
}

describe('RenderControlsPanel Font Assignment', () => {
  beforeEach(() => {
    useSelectionStore.getState().setPage('p1')
    useSelectionStore.getState().clear()
    usePreferencesStore.getState().setDefaultFont('Arial')
    useEditorUiStore.getState().clearError()
    useEditorUiStore.getState().setSelectedLanguage(undefined)
    useJobsStore.getState().clear()
    vi.clearAllMocks()

    server.use(
      http.get('/api/v1/config', () =>
        HttpResponse.json({ pipeline: { source_text_policy: 'han_only' } }),
      ),
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

  it('disables render controls while a pipeline job is running', async () => {
    useJobsStore.getState().started('job-1', 'pipeline')
    renderWithQuery(<RenderControlsPanel />)

    expect(await screen.findByTestId('render-font-select')).toBeDisabled()
  })

  it('refreshes the scene and skips stale font target ids', async () => {
    let sceneRequests = 0
    server.use(
      http.get('/api/v1/scene.json', () => {
        sceneRequests += 1
        return HttpResponse.json(
          sceneRequests === 1
            ? sceneWithTextNodes([
                { id: 't1', kind: { text: { style: { fontFamilies: ['Arial'] } } } },
              ])
            : sceneWithTextNodes([]),
        )
      }),
    )
    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t1', false)

    await userEvent.click(await screen.findByTestId('render-font-select'))
    await userEvent.click(await screen.findByText('Custom'))

    await waitFor(() => expect(sceneRequests).toBeGreaterThanOrEqual(2))
    expect(sceneActions.applyOp).not.toHaveBeenCalled()
    expect(sceneActions.runAutoRenderNow).not.toHaveBeenCalled()
  })

  it('handles style apply failures without queueing a render', async () => {
    vi.mocked(sceneActions.applyOp).mockRejectedValueOnce(new Error('node not found'))
    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t1', false)

    fireEvent.change(await screen.findByTestId('render-font-size'), {
      target: { value: '42' },
    })

    await waitFor(() =>
      expect(useEditorUiStore.getState().error?.message).toContain('node not found'),
    )
    expect(sceneActions.invalidateScene).toHaveBeenCalled()
    expect(sceneActions.queueAutoRender).not.toHaveBeenCalled()
    expect(sceneActions.runAutoRenderNow).not.toHaveBeenCalled()
  })

  it('does not request the online catalog when the picker mounts or opens', async () => {
    let catalogRequests = 0
    server.use(
      http.get('/api/v1/google-fonts', () => {
        catalogRequests += 1
        return HttpResponse.json({ fonts: [] })
      }),
    )

    renderWithQuery(<RenderControlsPanel />)
    await userEvent.click(await screen.findByTestId('render-font-select'))

    expect(catalogRequests).toBe(0)
  })

  it('requests the online catalog once after search intent', async () => {
    let catalogRequests = 0
    server.use(
      http.get('/api/v1/google-fonts', () => {
        catalogRequests += 1
        return HttpResponse.json({
          fonts: [
            {
              family: 'Online Sans',
              category: 'sans-serif',
              subsets: ['latin'],
              variants: [{ weight: 400, style: 'normal', filename: 'OnlineSans-Regular.ttf' }],
            },
          ],
        })
      }),
    )

    const { client } = renderWithQuery(<RenderControlsPanel />)
    await userEvent.click(await screen.findByTestId('render-font-select'))
    expect(catalogRequests).toBe(0)

    const search = screen.getByPlaceholderText('Search fonts…')
    await userEvent.type(search, 'Online')
    await screen.findByText('Online Sans')
    await userEvent.type(search, ' Sans')

    expect(catalogRequests).toBe(1)
    expect(client.getQueriesData({ queryKey: getGetGoogleFontsCatalogQueryKey() })).toHaveLength(1)
  })

  it('requests the online catalog once after Google category intent', async () => {
    let catalogRequests = 0
    server.use(
      http.get('/api/v1/google-fonts', () => {
        catalogRequests += 1
        return HttpResponse.json({ fonts: [] })
      }),
    )

    renderWithQuery(<RenderControlsPanel />)
    await userEvent.click(await screen.findByTestId('render-font-select'))
    expect(catalogRequests).toBe(0)

    await userEvent.click(screen.getByRole('button', { name: 'Sans' }))
    await waitFor(() => expect(catalogRequests).toBe(1))
    await userEvent.click(screen.getByRole('button', { name: 'Serif' }))

    expect(catalogRequests).toBe(1)
  })

  it('does not retry the online catalog after its first failed request', async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retryDelay: 0 } },
    })
    let catalogRequests = 0
    server.use(
      http.get('/api/v1/google-fonts', () => {
        catalogRequests += 1
        return HttpResponse.json({ message: 'catalog unavailable' }, { status: 500 })
      }),
    )

    renderWithQuery(<RenderControlsPanel />, { client })
    await userEvent.click(await screen.findByTestId('render-font-select'))
    await userEvent.type(screen.getByPlaceholderText('Search fonts…'), 'Online')

    await waitFor(() =>
      expect(client.getQueryState(getGetGoogleFontsCatalogQueryKey())?.status).toBe('error'),
    )
    expect(catalogRequests).toBe(1)
  })

  it('keeps a failed online catalog attempt terminal across lifecycle events and remount', async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retryDelay: 0 } },
    })
    let catalogRequests = 0
    server.use(
      http.get('/api/v1/google-fonts', () => {
        catalogRequests += 1
        return HttpResponse.json({ message: 'catalog unavailable' }, { status: 500 })
      }),
    )

    focusManager.setFocused(false)
    try {
      const firstRender = renderWithQuery(<RenderControlsPanel />, { client })
      await userEvent.click(await screen.findByTestId('render-font-select'))
      await userEvent.type(screen.getByPlaceholderText('Search fonts…'), 'Online')
      await waitFor(() =>
        expect(client.getQueryState(getGetGoogleFontsCatalogQueryKey())?.status).toBe('error'),
      )

      await userEvent.click(screen.getByRole('button', { name: 'Serif' }))
      focusManager.setFocused(true)
      onlineManager.setOnline(false)
      onlineManager.setOnline(true)
      await new Promise((resolve) => setTimeout(resolve, 50))

      firstRender.unmount()
      renderWithQuery(<RenderControlsPanel />, { client })
      await userEvent.click(await screen.findByTestId('render-font-select'))
      await userEvent.type(screen.getByPlaceholderText('Search fonts…'), 'Online')
      await userEvent.click(screen.getByRole('button', { name: 'Sans' }))
      await new Promise((resolve) => setTimeout(resolve, 50))

      expect(catalogRequests).toBe(1)
    } finally {
      focusManager.setFocused(undefined)
      onlineManager.setOnline(true)
    }
  })

  it('does not restart an in-flight online catalog request after unmount and new intent', async () => {
    const client = new QueryClient()
    let catalogRequests = 0
    let releaseCatalog: (() => void) | undefined
    const catalogDelay = new Promise<void>((resolve) => {
      releaseCatalog = resolve
    })
    server.use(
      http.get('/api/v1/google-fonts', async () => {
        catalogRequests += 1
        await catalogDelay
        return HttpResponse.json({ fonts: [] })
      }),
    )

    try {
      const firstRender = renderWithQuery(<RenderControlsPanel />, { client })
      await userEvent.click(await screen.findByTestId('render-font-select'))
      await userEvent.type(screen.getByPlaceholderText('Search fonts…'), 'Online')
      await waitFor(() => expect(catalogRequests).toBe(1))

      firstRender.unmount()
      renderWithQuery(<RenderControlsPanel />, { client })
      await userEvent.click(await screen.findByTestId('render-font-select'))
      await userEvent.type(screen.getByPlaceholderText('Search fonts…'), 'Online')
      await userEvent.click(screen.getByRole('button', { name: 'Sans' }))
      await new Promise((resolve) => setTimeout(resolve, 50))

      expect(catalogRequests).toBe(1)
    } finally {
      releaseCatalog?.()
    }
  })

  it('keeps the loaded online catalog across focus, reconnect, remount, and repeated intent', async () => {
    const client = new QueryClient()
    let catalogRequests = 0
    server.use(
      http.get('/api/v1/google-fonts', () => {
        catalogRequests += 1
        return HttpResponse.json({
          fonts: [
            {
              family: 'Online Sans',
              category: 'sans-serif',
              subsets: ['latin'],
              variants: [{ weight: 400, style: 'normal', filename: 'OnlineSans-Regular.ttf' }],
            },
          ],
        })
      }),
    )

    focusManager.setFocused(false)
    try {
      const firstRender = renderWithQuery(<RenderControlsPanel />, { client })
      await userEvent.click(await screen.findByTestId('render-font-select'))
      const search = screen.getByPlaceholderText('Search fonts…')
      await userEvent.type(search, 'Online')
      await screen.findByText('Online Sans')
      await userEvent.click(screen.getByRole('button', { name: 'Serif' }))

      focusManager.setFocused(true)
      await new Promise((resolve) => setTimeout(resolve, 50))
      onlineManager.setOnline(false)
      onlineManager.setOnline(true)
      await new Promise((resolve) => setTimeout(resolve, 50))

      firstRender.unmount()
      renderWithQuery(<RenderControlsPanel />, { client })
      await userEvent.click(await screen.findByTestId('render-font-select'))
      await userEvent.type(screen.getByPlaceholderText('Search fonts…'), 'Online')
      await screen.findByText('Online Sans')
      await new Promise((resolve) => setTimeout(resolve, 50))

      expect(catalogRequests).toBe(1)
    } finally {
      focusManager.setFocused(undefined)
      onlineManager.setOnline(true)
    }
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
    const op: Op = vi.mocked(sceneActions.applyOp).mock.calls[0][0]
    if (!('batch' in op)) throw new Error('expected a batch op')
    expect(op.batch.ops).toHaveLength(2)
    const [first, second] = op.batch.ops
    if (!first || !('updateNode' in first) || !second || !('updateNode' in second)) {
      throw new Error('expected updateNode ops')
    }
    const firstData = first.updateNode.patch.data
    const secondData = second.updateNode.patch.data
    if (!firstData || !('text' in firstData) || !secondData || !('text' in secondData)) {
      throw new Error('expected text patches')
    }
    expect(firstData.text.style).toMatchObject({
      fontFamilies: ['Custom'],
      fontSize: 18,
      color: [1, 2, 3, 255],
      effect: { bold: true, italic: false },
      stroke: { enabled: true, color: [7, 8, 9, 255], widthPx: 2 },
      textAlign: 'left',
    })
    expect(secondData.text.style).toMatchObject({
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
    const op: Op = vi.mocked(sceneActions.applyOp).mock.calls[0][0]
    if (!('batch' in op)) throw new Error('expected a batch op')
    expect(op.batch.ops).toHaveLength(2)
    for (const child of op.batch.ops) {
      if (!('updateNode' in child)) throw new Error('expected an updateNode op')
      const data = child.updateNode.patch.data
      if (!data || !('text' in data)) throw new Error('expected a text patch')
      expect(data.text.style?.fontFamilies).toEqual(['Roboto-Bold'])
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

  it('downloads a recommended uncached font and refreshes fonts before applying it', async () => {
    const events: string[] = []
    let fontRequests = 0
    server.use(
      http.get('/api/v1/fonts', () => {
        fontRequests += 1
        events.push(`fonts:${fontRequests}`)
        return HttpResponse.json([
          { familyName: 'Arial', postScriptName: 'Arial', source: 'system', cached: true },
          {
            familyName: 'Cloud',
            postScriptName: 'Cloud:400',
            source: 'google',
            cached: fontRequests > 1,
          },
        ])
      }),
      http.post('/api/v1/google-fonts/:family/fetch', () => {
        events.push('download')
        return new HttpResponse(null, { status: 204 })
      }),
    )
    vi.mocked(sceneActions.applyOp).mockImplementationOnce(async () => {
      events.push('apply')
    })
    vi.mocked(sceneActions.runAutoRenderNow).mockImplementationOnce(async () => {
      events.push('render')
    })

    renderWithQuery(<RenderControlsPanel />)
    await userEvent.click(await screen.findByTestId('render-font-select'))
    await userEvent.click(await screen.findByText('Cloud'))

    await waitFor(() => expect(events.at(-1)).toBe('render'))
    expect(fontRequests).toBe(2)
    expect(events).toEqual(['fonts:1', 'download', 'fonts:2', 'apply', 'render'])
    expect(sceneActions.applyOp).toHaveBeenCalledTimes(1)
    expect(sceneActions.runAutoRenderNow).toHaveBeenCalledTimes(1)
  })

  it('stops before applying or rendering when the fonts refresh fails', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
    let fontRequests = 0
    server.use(
      http.get('/api/v1/fonts', () => {
        fontRequests += 1
        if (fontRequests > 1) {
          return HttpResponse.json({ message: 'font list refresh failed' }, { status: 500 })
        }
        return HttpResponse.json([
          { familyName: 'Arial', postScriptName: 'Arial', source: 'system', cached: true },
          {
            familyName: 'Cloud',
            postScriptName: 'Cloud:400',
            source: 'google',
            cached: false,
          },
        ])
      }),
      http.post(
        '/api/v1/google-fonts/:family/fetch',
        () => new HttpResponse(null, { status: 204 }),
      ),
    )

    renderWithQuery(<RenderControlsPanel />)
    await userEvent.click(await screen.findByTestId('render-font-select'))
    await userEvent.click(await screen.findByText('Cloud'))

    await waitFor(
      () =>
        expect(useEditorUiStore.getState().error?.message).toContain('font list refresh failed'),
      { timeout: 3_000 },
    )
    expect(usePreferencesStore.getState().defaultFont).toBe('Arial')
    expect(sceneActions.applyOp).not.toHaveBeenCalled()
    expect(sceneActions.runAutoRenderNow).not.toHaveBeenCalled()
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

  it('uses the detected English size before prediction and subtracts five', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(
          sceneWithTextNodes([
            {
              id: 't1',
              kind: {
                text: {
                  style: { fontFamilies: ['Arial'] },
                  text: '中文',
                  translation: 'translated',
                  fontPrediction: { fontSizePx: 66 },
                  detectedFontSizePx: 60,
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
    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '55',
    )
    expect(input.value).toBe('')
    expect(sceneActions.applyOp).not.toHaveBeenCalled()
  })

  it('treats a trusted legacy Planner font size as automatic', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(
          sceneWithTextNodes([
            {
              id: 't1',
              kind: {
                text: {
                  style: { fontFamilies: ['Arial'], fontSize: 18 },
                  text: '中文',
                  translation: 'translated',
                  typographyPlanVerified: true,
                  fontPrediction: { fontSizePx: 60 },
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
    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '55',
    )
    expect(input.value).toBe('')
  })

  it('keeps the persisted Planner size outside recognized HanOnly mode', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    server.use(
      http.get('/api/v1/config', () =>
        HttpResponse.json({ pipeline: { source_text_policy: 'all_text' } }),
      ),
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(
          sceneWithTextNodes([
            {
              id: 't1',
              kind: {
                text: {
                  style: { fontFamilies: ['Arial'], fontSize: 18 },
                  typographyPlanVerified: true,
                  fontPrediction: { fontSizePx: 60 },
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
    await waitFor(() => expect(input.value).toBe('18'))
    expect(screen.queryByTestId('render-font-size-auto-hint')).not.toBeInTheDocument()
  })

  it.each(['ja-JP', 'ko-KR'])(
    'shows the unshifted HanOnly automatic size for %s',
    async (language) => {
      useEditorUiStore.getState().setSelectedLanguage(language)
      server.use(
        http.get('/api/v1/scene.json', () =>
          HttpResponse.json(
            sceneWithTextNodes([
              {
                id: 't1',
                kind: {
                  text: {
                    style: { fontFamilies: ['Arial'] },
                    text: '中文',
                    translation: 'translated',
                    fontPrediction: { fontSizePx: 60 },
                  },
                },
              },
            ]),
          ),
        ),
      )

      renderWithQuery(<RenderControlsPanel />)
      useSelectionStore.getState().select('t1', false)

      expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
        'data-size',
        '60',
      )
    },
  )

  it('subtracts five for another backend-recognized language', async () => {
    useEditorUiStore.getState().setSelectedLanguage('fr-FR')
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(
          sceneWithTextNodes([
            {
              id: 't1',
              kind: {
                text: {
                  style: { fontFamilies: ['Arial'] },
                  text: '中文',
                  translation: 'translated',
                  fontPrediction: { fontSizePx: 60 },
                },
              },
            },
          ]),
        ),
      ),
    )

    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t1', false)

    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '55',
    )
  })

  it.each(['German', 'Simplified Chinese'])(
    'recognizes the backend language name %s',
    async (language) => {
      useEditorUiStore.getState().setSelectedLanguage(language)
      server.use(
        http.get('/api/v1/scene.json', () =>
          HttpResponse.json(
            sceneWithTextNodes([
              {
                id: 't1',
                kind: {
                  text: {
                    style: { fontFamilies: ['Arial'] },
                    text: '中文',
                    translation: 'translated',
                    fontPrediction: { fontSizePx: 60 },
                  },
                },
              },
            ]),
          ),
        ),
      )

      renderWithQuery(<RenderControlsPanel />)
      useSelectionStore.getState().select('t1', false)

      expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
        'data-size',
        '55',
      )
    },
  )

  it('uses prediction when the detected English size is invalid', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en')
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(
          sceneWithTextNodes([
            {
              id: 't1',
              kind: {
                text: {
                  style: { fontFamilies: ['Arial'] },
                  text: '中文',
                  translation: 'translated',
                  fontPrediction: { fontSizePx: 60 },
                  detectedFontSizePx: 0,
                },
              },
            },
          ]),
        ),
      ),
    )

    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t1', false)

    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '55',
    )
  })

  it('keeps a manual size in the input and hides the automatic hint', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en')
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(
          sceneWithTextNodes([
            {
              id: 't1',
              kind: {
                text: {
                  style: { fontFamilies: ['Arial'], fontSize: 42 },
                  typographyPlanVerified: false,
                  fontPrediction: { fontSizePx: 60 },
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
    await waitFor(() => expect(input.value).toBe('42'))
    expect(screen.queryByTestId('render-font-size-auto-hint')).not.toBeInTheDocument()
  })

  it('shows the minimum automatic candidate for every node in a horizontal Source group', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithTextNodes(englishAutomaticGroupNodes())),
      ),
    )

    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t2', false)

    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '32.2',
    )
  })

  it('excludes an untranslated same-row node from the automatic group minimum', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    const nodes = englishAutomaticGroupNodes().slice(0, 2)
    nodes[1].kind.text.translation = '   '
    nodes[1].kind.text.detectedFontSizePx = 10
    server.use(http.get('/api/v1/scene.json', () => HttpResponse.json(sceneWithTextNodes(nodes))))

    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t2', false)

    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '71.8',
    )
  })

  it('uses the grouped automatic size as the first adjustment base', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithTextNodes(englishAutomaticGroupNodes())),
      ),
    )

    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t2', false)
    await screen.findByTestId('render-font-size-auto-hint')
    await userEvent.click(screen.getByTestId('render-font-size-decrease'))

    await waitFor(() => expect(sceneActions.applyOp).toHaveBeenCalledTimes(1))
    const op = vi.mocked(sceneActions.applyOp).mock.calls[0][0] as any
    expect(op.updateNode.patch.data.text.style.fontSize).toBe(31.2)
  })

  it('keeps a different row out of horizontal automatic grouping', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    const nodes = englishAutomaticGroupNodes()
    nodes[0].transform = { x: 120, y: 80, width: 100, height: 20, rotationDeg: 0 }
    server.use(http.get('/api/v1/scene.json', () => HttpResponse.json(sceneWithTextNodes(nodes))))

    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t2', false)

    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '71.8',
    )
  })

  it('keeps a vertical recognized node automatic but outside horizontal components', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    const nodes = englishAutomaticGroupNodes()
    nodes[2].kind.text.sourceDirection = 'vertical'
    nodes[2].kind.text.detectedFontSizePx = 60
    nodes.push({
      id: 'vertical-low',
      transform: { x: 360, y: 0, width: 100, height: 20, rotationDeg: 0 },
      kind: {
        text: {
          style: { fontFamilies: ['Arial'] },
          text: '中文',
          translation: 'vertical',
          sourceDirection: 'vertical',
          detectedFontSizePx: 10,
        },
      },
    })
    server.use(http.get('/api/v1/scene.json', () => HttpResponse.json(sceneWithTextNodes(nodes))))

    const view = renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t1', false)
    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '55',
    )

    useSelectionStore.getState().select('t2', false)
    view.rerender(<RenderControlsPanel />)
    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '32.2',
    )
  })

  it('excludes manual nodes from the automatic group minimum', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    const nodes = englishAutomaticGroupNodes()
    nodes[2].kind.text.style.fontSize = 10
    nodes[2].kind.text.typographyPlanVerified = false
    nodes[2].kind.text.detectedFontSizePx = 10
    server.use(http.get('/api/v1/scene.json', () => HttpResponse.json(sceneWithTextNodes(nodes))))

    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t2', false)

    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '32.2',
    )
  })

  it('preserves fractional automatic size in the hint and first adjustments', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(
          sceneWithTextNodes([
            {
              id: 't1',
              kind: {
                text: {
                  style: { fontFamilies: ['Arial'] },
                  text: '中文',
                  translation: 'translated',
                  fontPrediction: { fontSizePx: 60.4 },
                },
              },
            },
          ]),
        ),
      ),
    )

    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t1', false)

    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '55.4',
    )
    await userEvent.click(screen.getByTestId('render-font-size-decrease'))

    await waitFor(() => expect(sceneActions.applyOp).toHaveBeenCalledTimes(1))
    let op = vi.mocked(sceneActions.applyOp).mock.calls[0][0] as any
    expect(op.updateNode.id).toBe('t1')
    expect(op.updateNode.patch.data.text.style.fontSize).toBe(54.4)

    vi.mocked(sceneActions.applyOp).mockClear()
    await userEvent.click(screen.getByTestId('render-font-size-increase'))

    await waitFor(() => expect(sceneActions.applyOp).toHaveBeenCalledTimes(1))
    op = vi.mocked(sceneActions.applyOp).mock.calls[0][0] as any
    expect(op.updateNode.id).toBe('t1')
    expect(op.updateNode.patch.data.text.style.fontSize).toBe(56.4)
  })

  it('excludes stale protected and non-Han nodes from the automatic group', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    const nodes = [
      {
        id: 'selected',
        transform: { x: 0, y: 0, width: 100, height: 50, rotationDeg: 0 },
        kind: {
          text: {
            style: { fontFamilies: ['Arial'] },
            text: '中文',
            translation: 'translated',
            detectedFontSizePx: 60,
          },
        },
      },
      {
        id: 'protected',
        transform: { x: 120, y: 0, width: 100, height: 50, rotationDeg: 0 },
        kind: {
          text: {
            style: { fontFamilies: ['Arial'] },
            text: 'Slim Waist 中文',
            translation: 'stale',
            detectedFontSizePx: 8,
          },
        },
      },
      {
        id: 'latin',
        transform: { x: 240, y: 0, width: 100, height: 50, rotationDeg: 0 },
        kind: {
          text: {
            style: { fontFamilies: ['Arial'] },
            text: 'Peach',
            translation: 'stale',
            detectedFontSizePx: 7,
          },
        },
      },
    ]
    server.use(http.get('/api/v1/scene.json', () => HttpResponse.json(sceneWithTextNodes(nodes))))

    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('selected', false)

    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '55',
    )
  })

  it('rejects a retraced zero-area line quad from source-relative sizing and grouping', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    const nodes = [
      {
        id: 'selected',
        transform: { x: 0, y: 0, width: 100, height: 50, rotationDeg: 0 },
        kind: {
          text: {
            style: { fontFamilies: ['Arial'] },
            text: '中文',
            translation: 'translated',
            detectedFontSizePx: 60,
          },
        },
      },
      {
        id: 'retraced',
        transform: { x: 120, y: 0, width: 100, height: 50, rotationDeg: 0 },
        kind: {
          text: {
            style: { fontFamilies: ['Arial'] },
            text: '中文\nPeach',
            translation: 'stale',
            linePolygons: [
              [
                [120, 0],
                [121, 0],
                [120, 0],
                [120, 1],
              ],
              [
                [120, 20],
                [220, 20],
                [220, 40],
                [120, 40],
              ],
            ],
            detectedFontSizePx: 6,
          },
        },
      },
    ]
    server.use(http.get('/api/v1/scene.json', () => HttpResponse.json(sceneWithTextNodes(nodes))))

    const view = renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('retraced', false)
    await screen.findByTestId('render-font-size')
    expect(screen.queryByTestId('render-font-size-auto-hint')).not.toBeInTheDocument()

    useSelectionStore.getState().select('selected', false)
    view.rerender(<RenderControlsPanel />)
    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '55',
    )
  })

  it('rejects a tiny positive-area line quad below the Rust f32 epsilon', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    const nodes = [
      {
        id: 'selected',
        transform: { x: 0, y: 0, width: 100, height: 50, rotationDeg: 0 },
        kind: {
          text: {
            style: { fontFamilies: ['Arial'] },
            text: '中文',
            translation: 'translated',
            detectedFontSizePx: 60,
          },
        },
      },
      {
        id: 'tiny',
        transform: { x: 120, y: 0, width: 100, height: 50, rotationDeg: 0 },
        kind: {
          text: {
            style: { fontFamilies: ['Arial'] },
            text: '中文\nPeach',
            translation: 'stale',
            linePolygons: [
              [
                [120, 0],
                [121, 0],
                [121, 0.00000005],
                [120, 0.00000005],
              ],
              [
                [120, 20],
                [220, 20],
                [220, 40],
                [120, 40],
              ],
            ],
            detectedFontSizePx: 6,
          },
        },
      },
    ]
    server.use(http.get('/api/v1/scene.json', () => HttpResponse.json(sceneWithTextNodes(nodes))))

    const view = renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('tiny', false)
    await screen.findByTestId('render-font-size')
    expect(screen.queryByTestId('render-font-size-auto-hint')).not.toBeInTheDocument()

    useSelectionStore.getState().select('selected', false)
    view.rerender(<RenderControlsPanel />)
    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '55',
    )
  })

  it('uses the eligible Han box fallback as the automatic hint and adjustment base', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(
          sceneWithTextNodes([
            {
              id: 't1',
              transform: { x: 0, y: 0, width: 100, height: 50, rotationDeg: 0 },
              kind: {
                text: {
                  style: { fontFamilies: ['Arial'] },
                  text: '中文',
                  translation: 'translated',
                },
              },
            },
          ]),
        ),
      ),
    )

    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t1', false)

    expect(await screen.findByTestId('render-font-size-auto-hint')).toHaveAttribute(
      'data-size',
      '17.5',
    )
    await userEvent.click(screen.getByTestId('render-font-size-decrease'))
    await waitFor(() => expect(sceneActions.applyOp).toHaveBeenCalledTimes(1))
    const op = vi.mocked(sceneActions.applyOp).mock.calls[0][0] as any
    expect(op.updateNode.patch.data.text.style.fontSize).toBe(16.5)
  })

  it('keeps legacy sizing while config is loading and remains legacy for eventual AllText', async () => {
    useEditorUiStore.getState().setSelectedLanguage('en-US')
    queryClient.removeQueries({ queryKey: getGetConfigQueryKey() })
    server.use(
      http.get('/api/v1/config', async () => {
        await delay(100)
        return HttpResponse.json({ pipeline: { source_text_policy: 'all_text' } })
      }),
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(
          sceneWithTextNodes([
            {
              id: 't1',
              kind: {
                text: {
                  style: { fontFamilies: ['Arial'] },
                  text: '中文',
                  translation: 'translated',
                  detectedFontSizePx: 60,
                },
              },
            },
          ]),
        ),
      ),
    )

    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().select('t1', false)
    await screen.findByTestId('render-font-size')
    expect(screen.queryByTestId('render-font-size-auto-hint')).not.toBeInTheDocument()
    await userEvent.click(screen.getByTestId('render-font-size-increase'))
    await waitFor(() => expect(sceneActions.applyOp).toHaveBeenCalledTimes(1))
    const op = vi.mocked(sceneActions.applyOp).mock.calls[0][0] as any
    expect(op.updateNode.patch.data.text.style.fontSize).toBe(17)
    await delay(120)
    expect(screen.queryByTestId('render-font-size-auto-hint')).not.toBeInTheDocument()
  })

  it.each([
    ['AllText', 'all_text', 'en', 60],
    ['missing language', 'han_only', undefined, 60],
    ['missing metadata', 'han_only', 'en', undefined],
    ['invalid language code', 'han_only', 'xx', 60],
    ['invalid language name', 'han_only', 'not-a-language', 60],
  ])(
    'keeps blank automatic size and the 16 px adjustment base for %s',
    async (_name, sourceTextPolicy, language, predictedSize) => {
      useEditorUiStore.getState().setSelectedLanguage(language)
      server.use(
        http.get('/api/v1/config', () =>
          HttpResponse.json({ pipeline: { source_text_policy: sourceTextPolicy } }),
        ),
        http.get('/api/v1/scene.json', () =>
          HttpResponse.json(
            sceneWithTextNodes([
              {
                id: 't1',
                kind: {
                  text: {
                    style: { fontFamilies: ['Arial'] },
                    ...(predictedSize === undefined
                      ? {}
                      : { fontPrediction: { fontSizePx: predictedSize } }),
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
      expect(screen.queryByTestId('render-font-size-auto-hint')).not.toBeInTheDocument()
      await userEvent.click(screen.getByTestId('render-font-size-increase'))

      await waitFor(() => expect(sceneActions.applyOp).toHaveBeenCalledTimes(1))
      const op = vi.mocked(sceneActions.applyOp).mock.calls[0][0] as any
      expect(op.updateNode.patch.data.text.style.fontSize).toBe(17)
    },
  )

  it('keeps the automatic size interpolation in both owned locales', () => {
    expect(enUsTranslation.render.fontSizeAutoHint).toContain('{{size}}')
    expect(zhCnTranslation.render.fontSizeAutoHint).toContain('{{size}}')
    expect(enUsTranslation.render.fontSizeAutoHint.replace('{{size}}', '58')).toBe('auto 58 px')
    expect(zhCnTranslation.render.fontSizeAutoHint.replace('{{size}}', '58')).toBe('自动 58 px')
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

  it('batch font update preserves every other explicit style field', async () => {
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithTextNodes(fullyStyledTextNodes())),
      ),
    )
    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().selectMany(['t1', 't2', 't3'])

    await userEvent.click(await screen.findByTestId('render-font-select'))
    await userEvent.click(await screen.findByText('Roboto'))

    await waitFor(() => expect(sceneActions.applyOp).toHaveBeenCalledTimes(1))
    expectBatchStyleUpdate({ fontFamilies: ['Roboto'] })
    expect(sceneActions.runAutoRenderNow).toHaveBeenCalledTimes(1)
    expect(sceneActions.runAutoRenderNow).toHaveBeenCalledWith('p1')
    expect(sceneActions.queueAutoRender).not.toHaveBeenCalled()
  })

  it('batch font-size update preserves every other explicit style field', async () => {
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithTextNodes(fullyStyledTextNodes())),
      ),
    )
    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().selectMany(['t1', 't2', 't3'])

    fireEvent.change(await screen.findByTestId('render-font-size'), {
      target: { value: '42' },
    })

    await waitFor(() => expect(sceneActions.applyOp).toHaveBeenCalledTimes(1))
    expectBatchStyleUpdate({ fontSize: 42 })
    expect(sceneActions.queueAutoRender).toHaveBeenCalledTimes(1)
    expect(sceneActions.queueAutoRender).toHaveBeenCalledWith('p1')
    expect(sceneActions.runAutoRenderNow).not.toHaveBeenCalled()
  })

  it('batch color update preserves every other explicit style field', async () => {
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithTextNodes(fullyStyledTextNodes())),
      ),
    )
    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().selectMany(['t1', 't2', 't3'])

    await userEvent.click(await screen.findByTestId('render-color-trigger'))
    fireEvent.change(await screen.findByTestId('render-color-input'), {
      target: { value: '#123456' },
    })

    await waitFor(() => expect(sceneActions.applyOp).toHaveBeenCalledTimes(1))
    expectBatchStyleUpdate({ color: [18, 52, 86, 255] })
    expect(sceneActions.queueAutoRender).toHaveBeenCalledTimes(1)
    expect(sceneActions.queueAutoRender).toHaveBeenCalledWith('p1')
    expect(sceneActions.runAutoRenderNow).not.toHaveBeenCalled()
  })

  it('batch stroke update applies one complete stroke and preserves other style fields', async () => {
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithTextNodes(fullyStyledTextNodes())),
      ),
    )
    renderWithQuery(<RenderControlsPanel />)
    useSelectionStore.getState().selectMany(['t1', 't2', 't3'])

    fireEvent.change(await screen.findByTestId('render-stroke-width'), {
      target: { value: '5.5' },
    })

    await waitFor(() => expect(sceneActions.applyOp).toHaveBeenCalledTimes(1))
    expectBatchStyleUpdate({
      stroke: { enabled: true, color: [7, 8, 9, 255], widthPx: 5.5 },
    })
    expect(sceneActions.queueAutoRender).toHaveBeenCalledTimes(1)
    expect(sceneActions.queueAutoRender).toHaveBeenCalledWith('p1')
    expect(sceneActions.runAutoRenderNow).not.toHaveBeenCalled()
  })
})
