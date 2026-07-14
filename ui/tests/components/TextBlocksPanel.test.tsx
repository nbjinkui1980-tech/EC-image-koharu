import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { http, HttpResponse } from 'msw'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { TextBlocksPanel } from '@/components/panels/TextBlocksPanel'
import * as sceneActions from '@/lib/io/scene'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'
import { useJobsStore } from '@/lib/stores/jobsStore'
import { usePreferencesStore } from '@/lib/stores/preferencesStore'
import { useSelectionStore } from '@/lib/stores/selectionStore'

import { renderWithQuery } from '../helpers'
import { readyLlmState } from '../msw/fixtures'
import { server } from '../msw/server'

vi.mock('@/lib/io/scene', async () => {
  const actual = await vi.importActual<typeof import('@/lib/io/scene')>('@/lib/io/scene')
  return {
    ...actual,
    applyOp: vi.fn(),
    queueAutoRender: vi.fn(),
  }
})

function sceneWithTextNodes() {
  return {
    epoch: 1,
    scene: {
      pages: {
        p1: {
          id: 'p1',
          name: 'P1',
          width: 100,
          height: 100,
          nodes: {
            t1: {
              id: 't1',
              transform: { x: 0, y: 0, width: 10, height: 10, rotationDeg: 0 },
              visible: true,
              kind: { text: { text: 'first' } },
            },
            t2: {
              id: 't2',
              transform: { x: 10, y: 10, width: 10, height: 10, rotationDeg: 0 },
              visible: true,
              kind: { text: { text: 'second' } },
            },
          },
        },
      },
      project: { name: 'Proj' },
    },
  }
}

function sceneWithSingleTextNode(text: string, linePolygons?: number[][][]) {
  const scene = sceneWithTextNodes()
  scene.scene.pages.p1.nodes = {
    t1: {
      id: 't1',
      transform: { x: 0, y: 0, width: 80, height: 40, rotationDeg: 0 },
      visible: true,
      kind: { text: { text, linePolygons } },
    },
  }
  return scene
}

describe('TextBlocksPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    useSelectionStore.getState().setPage('p1')
    useSelectionStore.getState().select('t2', false)
    useJobsStore.getState().clear()
    useEditorUiStore.setState({ selectedLanguage: 'en' })
    usePreferencesStore.setState({
      customSystemPrompt: 'translate naturally',
      defaultFont: 'Arial',
    })
  })

  it('clears OCR line polygons when the OCR text is edited', async () => {
    useSelectionStore.getState().select('t1', false)
    const polygons = [
      [
        [0, 0],
        [40, 0],
        [40, 20],
        [0, 20],
      ],
      [
        [40, 0],
        [80, 0],
        [80, 20],
        [40, 20],
      ],
    ]
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithSingleTextNode('Peach\n蜜桃臀', polygons)),
      ),
      http.get('/api/v1/config', () =>
        HttpResponse.json({ pipeline: { source_text_policy: 'han_only' } }),
      ),
      http.get('/api/v1/llm/current', () => HttpResponse.json(readyLlmState)),
    )

    renderWithQuery(<TextBlocksPanel />)
    const textarea = await screen.findByTestId('textblock-ocr-0')
    await userEvent.clear(textarea)
    await userEvent.type(textarea, '蜜桃臀\nPeach')

    const calls = vi.mocked(sceneActions.applyOp).mock.calls
    const lastOp = calls.at(-1)?.[0] as any
    expect(lastOp.updateNode.patch.data.text).toMatchObject({
      text: '蜜桃臀\nPeach',
      linePolygons: null,
      translation: null,
      sprite: null,
      spriteTransform: null,
      renderedDirection: null,
    })
  })

  it('generates translation only for the clicked text block', async () => {
    const pipelineRequests: unknown[] = []
    server.use(
      http.get('/api/v1/scene.json', () => HttpResponse.json(sceneWithTextNodes())),
      http.get('/api/v1/config', () =>
        HttpResponse.json({
          pipeline: {
            translator: 'llm',
            renderer: 'koharu-renderer',
            source_text_policy: 'all_text',
          },
        }),
      ),
      http.get('/api/v1/llm/current', () => HttpResponse.json(readyLlmState)),
      http.post('/api/v1/pipelines', async ({ request }) => {
        pipelineRequests.push(await request.json())
        return HttpResponse.json({ operationId: 'op-1' })
      }),
    )

    renderWithQuery(<TextBlocksPanel />)

    const generateButton = await screen.findByTestId('textblock-generate-1')
    await waitFor(() => expect(generateButton).not.toBeDisabled())
    await userEvent.click(generateButton)

    await waitFor(() => expect(pipelineRequests).toHaveLength(1))
    expect(pipelineRequests[0]).toMatchObject({
      steps: ['llm', 'koharu-renderer'],
      pages: ['p1'],
      textNodeIds: ['t2'],
      targetLanguage: 'en',
      systemPrompt: 'translate naturally',
      defaultFont: 'Arial',
    })
  })

  it('shows a safe-skip warning for HanOnly mixed text without line polygons', async () => {
    useSelectionStore.getState().select('t1', false)
    let pipelinePosts = 0
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithSingleTextNode('中文文案\nENGLISH COPY')),
      ),
      http.get('/api/v1/config', () => HttpResponse.json({ pipeline: {} })),
      http.get('/api/v1/llm/current', () => HttpResponse.json(readyLlmState)),
      http.post('/api/v1/pipelines', () => {
        pipelinePosts += 1
        return HttpResponse.json({ operationId: 'unexpected' })
      }),
    )

    renderWithQuery(<TextBlocksPanel />)

    expect(await screen.findByTestId('textblock-geometry-warning-0')).toBeInTheDocument()
    const generateButton = screen.getByTestId('textblock-generate-0')
    expect(generateButton).toBeDisabled()
    await userEvent.click(generateButton)
    expect(pipelinePosts).toBe(0)
  })

  it('shows a safe-skip warning for HanOnly mixed text with mismatched line polygons', async () => {
    useSelectionStore.getState().select('t1', false)
    const polygons = [
      [
        [0, 0],
        [40, 0],
        [40, 20],
        [0, 20],
      ],
    ]
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithSingleTextNode('中文文案\nENGLISH COPY', polygons)),
      ),
      http.get('/api/v1/config', () =>
        HttpResponse.json({ pipeline: { source_text_policy: 'han_only' } }),
      ),
      http.get('/api/v1/llm/current', () => HttpResponse.json(readyLlmState)),
    )

    renderWithQuery(<TextBlocksPanel />)

    expect(await screen.findByTestId('textblock-geometry-warning-0')).toBeInTheDocument()
    expect(screen.getByTestId('textblock-generate-0')).toBeDisabled()
  })

  it('disables Generate for pure non-Han text in HanOnly mode', async () => {
    useSelectionStore.getState().select('t1', false)
    let pipelinePosts = 0
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithSingleTextNode('ENGLISH COPY')),
      ),
      http.get('/api/v1/config', () =>
        HttpResponse.json({ pipeline: { source_text_policy: 'han_only' } }),
      ),
      http.get('/api/v1/llm/current', () => HttpResponse.json(readyLlmState)),
      http.post('/api/v1/pipelines', () => {
        pipelinePosts += 1
        return HttpResponse.json({ operationId: 'unexpected' })
      }),
    )

    renderWithQuery(<TextBlocksPanel />)

    const generateButton = await screen.findByTestId('textblock-generate-0')
    expect(generateButton).toBeDisabled()
    expect(screen.getByTestId('textblock-geometry-warning-0')).toBeInTheDocument()
    await userEvent.click(generateButton)
    expect(pipelinePosts).toBe(0)
  })

  it('disables Generate for unresolved inline English word plus Han text', async () => {
    useSelectionStore.getState().select('t1', false)
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithSingleTextNode('Peach蜜桃臀')),
      ),
      http.get('/api/v1/config', () =>
        HttpResponse.json({ pipeline: { source_text_policy: 'han_only' } }),
      ),
      http.get('/api/v1/llm/current', () => HttpResponse.json(readyLlmState)),
    )

    renderWithQuery(<TextBlocksPanel />)

    expect(await screen.findByTestId('textblock-generate-0')).toBeDisabled()
    expect(screen.getByTestId('textblock-geometry-warning-0')).toBeInTheDocument()
  })

  it('allows Generate for a single Latin label plus Han text', async () => {
    useSelectionStore.getState().select('t1', false)
    server.use(
      http.get('/api/v1/scene.json', () => HttpResponse.json(sceneWithSingleTextNode('S型曲线'))),
      http.get('/api/v1/config', () =>
        HttpResponse.json({ pipeline: { source_text_policy: 'han_only' } }),
      ),
      http.get('/api/v1/llm/current', () => HttpResponse.json(readyLlmState)),
    )

    renderWithQuery(<TextBlocksPanel />)

    const generateButton = await screen.findByTestId('textblock-generate-0')
    await waitFor(() => expect(generateButton).not.toBeDisabled())
    expect(screen.queryByTestId('textblock-geometry-warning-0')).not.toBeInTheDocument()
  })

  it('allows Generate after PP-OCRv5 separates English and Han units', async () => {
    useSelectionStore.getState().select('t1', false)
    const polygons = [
      [
        [0, 0],
        [40, 0],
        [40, 20],
        [0, 20],
      ],
      [
        [40, 0],
        [80, 0],
        [80, 20],
        [40, 20],
      ],
    ]
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithSingleTextNode('Peach\n蜜桃臀', polygons)),
      ),
      http.get('/api/v1/config', () =>
        HttpResponse.json({ pipeline: { source_text_policy: 'han_only' } }),
      ),
      http.get('/api/v1/llm/current', () => HttpResponse.json(readyLlmState)),
    )

    renderWithQuery(<TextBlocksPanel />)

    const generateButton = await screen.findByTestId('textblock-generate-0')
    await waitFor(() => expect(generateButton).not.toBeDisabled())
    expect(screen.queryByTestId('textblock-geometry-warning-0')).not.toBeInTheDocument()
  })

  it('allows HanOnly mixed text when every non-empty line has geometry', async () => {
    useSelectionStore.getState().select('t1', false)
    const polygons = [
      [
        [0, 0],
        [40, 0],
        [40, 20],
        [0, 20],
      ],
      [
        [0, 20],
        [40, 20],
        [40, 40],
        [0, 40],
      ],
    ]
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithSingleTextNode('中文文案\nENGLISH COPY', polygons)),
      ),
      http.get('/api/v1/config', () =>
        HttpResponse.json({ pipeline: { source_text_policy: 'han_only' } }),
      ),
      http.get('/api/v1/llm/current', () => HttpResponse.json(readyLlmState)),
    )

    renderWithQuery(<TextBlocksPanel />)

    const generateButton = await screen.findByTestId('textblock-generate-0')
    await waitFor(() => expect(generateButton).not.toBeDisabled())
    expect(screen.queryByTestId('textblock-geometry-warning-0')).not.toBeInTheDocument()
  })

  it('preserves legacy AllText behavior without recommending it in ecommerce UI', async () => {
    useSelectionStore.getState().select('t1', false)
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithSingleTextNode('中文文案\nENGLISH COPY')),
      ),
      http.get('/api/v1/config', () =>
        HttpResponse.json({ pipeline: { source_text_policy: 'all_text' } }),
      ),
      http.get('/api/v1/llm/current', () => HttpResponse.json(readyLlmState)),
    )

    renderWithQuery(<TextBlocksPanel />)

    const generateButton = await screen.findByTestId('textblock-generate-0')
    await waitFor(() => expect(generateButton).not.toBeDisabled())
    expect(screen.queryByTestId('textblock-geometry-warning-0')).not.toBeInTheDocument()
  })

  it('does not warn for pure Han text', async () => {
    useSelectionStore.getState().select('t1', false)
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json(sceneWithSingleTextNode('纯中文文案')),
      ),
      http.get('/api/v1/config', () =>
        HttpResponse.json({ pipeline: { source_text_policy: 'han_only' } }),
      ),
      http.get('/api/v1/llm/current', () => HttpResponse.json(readyLlmState)),
    )

    renderWithQuery(<TextBlocksPanel />)

    const generateButton = await screen.findByTestId('textblock-generate-0')
    await waitFor(() => expect(generateButton).not.toBeDisabled())
    expect(screen.queryByTestId('textblock-geometry-warning-0')).not.toBeInTheDocument()
  })
})
