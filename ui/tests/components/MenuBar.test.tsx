import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { http, HttpResponse } from 'msw'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { MenuBar } from '@/components/MenuBar'
import { getGetConfigQueryKey, getGetSceneJsonQueryKey } from '@/lib/api'
import { queryClient } from '@/lib/queryClient'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'
import { usePreferencesStore } from '@/lib/stores/preferencesStore'

import { renderWithQuery } from '../helpers'
import { server } from '../msw/server'

vi.mock('@/lib/io/openFiles', () => ({
  openImageFiles: vi.fn().mockResolvedValue([]),
  openImageFolder: vi.fn().mockResolvedValue([]),
  openKhrFile: vi.fn().mockResolvedValue(null),
}))

beforeEach(() => {
  usePreferencesStore.getState().resetPreferences()
  useEditorUiStore.setState({ selectedLanguage: undefined, readingOrder: 'rtl' })
  // Default: config + scene exist so the menu enables scene-dependent items.
  server.use(
    http.get('/api/v1/scene.json', () =>
      HttpResponse.json({
        epoch: 0,
        scene: { pages: {}, project: { name: 'P' } as never },
      }),
    ),
    http.get('/api/v1/config', () => HttpResponse.json({})),
  )
  queryClient.setQueryData(getGetSceneJsonQueryKey(), {
    epoch: 0,
    scene: { pages: {}, project: { name: 'P' } },
  })
  queryClient.setQueryData(getGetConfigQueryKey(), {})
})

describe('MenuBar', () => {
  it('renders File / View / Process / Help triggers', async () => {
    renderWithQuery(<MenuBar />)
    expect(screen.getByTestId('menu-file-trigger')).toBeInTheDocument()
    expect(screen.getByTestId('menu-process-trigger')).toBeInTheDocument()
  })

  it('Close Project calls DELETE /projects/current and invalidates scene', async () => {
    let deleted = 0
    server.use(
      http.delete('/api/v1/projects/current', () => {
        deleted += 1
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const invalidateSpy = vi.spyOn(queryClient, 'invalidateQueries')

    renderWithQuery(<MenuBar />)
    await userEvent.click(screen.getByTestId('menu-file-trigger'))
    const close = await screen.findByTestId('menu-file-close-project')
    await userEvent.click(close)

    await waitFor(() => expect(deleted).toBe(1))
    await waitFor(() => {
      const invalidatedKeys = invalidateSpy.mock.calls.map((c) => c[0]?.queryKey)
      expect(invalidatedKeys).toContainEqual(getGetSceneJsonQueryKey())
    })
  })

  it('Close Project is disabled when no project is open', async () => {
    // Clear seeded cache + point /scene.json at the 400 response so useScene
    // resolves to null.
    queryClient.clear()
    server.use(
      http.get('/api/v1/scene.json', () =>
        HttpResponse.json({ message: 'no project' }, { status: 400 }),
      ),
    )
    renderWithQuery(<MenuBar />)
    await waitFor(() => expect(queryClient.isFetching()).toBe(0))
    await userEvent.click(screen.getByTestId('menu-file-trigger'))
    const close = await screen.findByTestId('menu-file-close-project')
    expect(close).toHaveAttribute('data-disabled')
  })

  it('menu_hides_custom_pipeline_controls', async () => {
    renderWithQuery(<MenuBar />)
    await userEvent.click(screen.getByTestId('menu-process-trigger'))

    expect(screen.queryByText('menu.customPipeline')).not.toBeInTheDocument()
    expect(screen.queryByText('menu.runCustomCurrent')).not.toBeInTheDocument()
    expect(screen.queryByText('menu.runCustomAll')).not.toBeInTheDocument()
  })

  it('full_pipeline_includes_planner_only_when_enabled', async () => {
    usePreferencesStore.setState({
      customSystemPrompt: 'translate products',
      defaultFont: 'Inter',
    })
    useEditorUiStore.setState({ selectedLanguage: 'de-DE', readingOrder: 'ltr' })
    let request: Record<string, unknown> | undefined
    server.use(
      http.get('/api/v1/config', () =>
        HttpResponse.json({
          pipeline: {
            detector: 'detector',
            segmenter: 'segmenter',
            bubble_segmenter: 'bubble',
            font_detector: 'font',
            ocr: 'ocr',
            translator: 'translator',
            typography_planner: 'cloud-typography-planner',
            inpainter: 'inpainter',
            renderer: 'renderer',
          },
          typography_planner: { enabled: true },
        }),
      ),
      http.post('/api/v1/pipelines', async ({ request: incoming }) => {
        request = (await incoming.json()) as Record<string, unknown>
        return HttpResponse.json({ jobId: 'job' })
      }),
    )

    renderWithQuery(<MenuBar />)
    await userEvent.click(screen.getByTestId('menu-process-trigger'))
    await userEvent.click(await screen.findByText('menu.processAll'))

    await waitFor(() => {
      expect(request).toEqual(
        expect.objectContaining({
          targetLanguage: 'de-DE',
          systemPrompt: 'translate products',
          defaultFont: 'Inter',
          readingOrder: 'ltr',
        }),
      )
      expect(request?.steps).toEqual(
        expect.arrayContaining([
          'detector',
          'segmenter',
          'bubble',
          'font',
          'ocr',
          'translator',
          'cloud-typography-planner',
          'inpainter',
          'renderer',
        ]),
      )
      expect(request?.steps).toHaveLength(9)
    })
  })

  it('full_pipeline_omits_planner_when_disabled', async () => {
    let request: { steps?: string[] } | undefined
    server.use(
      http.get('/api/v1/config', () =>
        HttpResponse.json({
          pipeline: {
            detector: 'detector',
            segmenter: 'segmenter',
            bubble_segmenter: 'bubble',
            font_detector: 'font',
            ocr: 'ocr',
            translator: 'translator',
            typography_planner: 'cloud-typography-planner',
            inpainter: 'inpainter',
            renderer: 'renderer',
          },
          typography_planner: { enabled: false },
        }),
      ),
      http.post('/api/v1/pipelines', async ({ request: incoming }) => {
        request = (await incoming.json()) as { steps: string[] }
        return HttpResponse.json({ jobId: 'job' })
      }),
    )

    renderWithQuery(<MenuBar />)
    await userEvent.click(screen.getByTestId('menu-process-trigger'))
    await userEvent.click(await screen.findByText('menu.processAll'))

    await waitFor(() => expect(request).toBeDefined())
    expect(request?.steps).not.toContain('cloud-typography-planner')
  })
})
