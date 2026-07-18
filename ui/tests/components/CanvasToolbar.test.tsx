import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { http, HttpResponse } from 'msw'
import { beforeEach, describe, expect, it } from 'vitest'

import { ActivityBubble } from '@/components/ActivityBubble'
import { CanvasToolbar } from '@/components/canvas/CanvasToolbar'
import type { StartPipelineRequest } from '@/lib/api/schemas'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'
import { useJobsStore } from '@/lib/stores/jobsStore'
import { useSelectionStore } from '@/lib/stores/selectionStore'

import { renderWithQuery } from '../helpers'
import { readyLlmState } from '../msw/fixtures'
import { server } from '../msw/server'

beforeEach(() => {
  useEditorUiStore.getState().clearError()
  useJobsStore.getState().clear()
  useSelectionStore.getState().setPage('p1')
  server.use(
    http.get('/api/v1/llm/current', () => HttpResponse.json(readyLlmState)),
    http.get('/api/v1/llm/catalog', () => HttpResponse.json({ localModels: [], providers: [] })),
  )
})

describe('CanvasToolbar', () => {
  it('shows workflow actions in execution order', async () => {
    renderWithQuery(<CanvasToolbar />)

    const toolbar = (await screen.findByTestId('toolbar-detect')).parentElement
    expect(toolbar).not.toBeNull()
    expect(
      Array.from(toolbar!.querySelectorAll<HTMLButtonElement>('button')).map(
        (button) => button.dataset.testid,
      ),
    ).toEqual([
      'toolbar-detect',
      'toolbar-ocr',
      'toolbar-translate',
      'toolbar-inpaint',
      'toolbar-typography',
      'toolbar-render',
    ])
  })

  it('runs manual Smart Typography when automatic typography is disabled', async () => {
    const requests: StartPipelineRequest[] = []
    server.use(
      http.get('/api/v1/config', () =>
        HttpResponse.json({
          pipeline: {
            typography_planner: 'cloud-typography-planner',
            renderer: 'koharu-renderer',
          },
          providers: [{ id: 'openai-compatible', base_url: ' https://planner.test/v1 ' }],
          typography_planner: { enabled: false, model_id: ' planner-model ' },
        }),
      ),
      http.post('/api/v1/pipelines', async ({ request }) => {
        requests.push((await request.json()) as StartPipelineRequest)
        return HttpResponse.json({ operationId: 'typography-job' })
      }),
    )

    renderWithQuery(<CanvasToolbar />)
    await userEvent.click(await screen.findByTestId('toolbar-typography'))

    await waitFor(() => expect(requests).toHaveLength(1))
    expect(requests[0]).toMatchObject({
      steps: ['cloud-typography-planner', 'koharu-renderer'],
      pages: ['p1'],
    })
  })

  it('shows a configuration error without starting Smart Typography when the planner model is missing or blank', async () => {
    let modelId: string | undefined
    let pipelinePosts = 0
    server.use(
      http.get('/api/v1/config', () =>
        HttpResponse.json({
          pipeline: {
            typography_planner: 'cloud-typography-planner',
            renderer: 'koharu-renderer',
          },
          providers: [{ id: 'openai-compatible', base_url: 'https://planner.test/v1' }],
          typography_planner: { enabled: false, model_id: modelId },
        }),
      ),
      http.post('/api/v1/pipelines', () => {
        pipelinePosts += 1
        return HttpResponse.json({ operationId: 'unexpected' })
      }),
    )

    for (const unconfiguredModelId of [undefined, '   ']) {
      modelId = unconfiguredModelId
      const view = renderWithQuery(
        <>
          <CanvasToolbar />
          <ActivityBubble />
        </>,
      )
      await userEvent.click(await screen.findByTestId('toolbar-typography'))

      await waitFor(() => {
        expect(useEditorUiStore.getState().error?.message).toBe(
          'settings.typographyConnectionRequired',
        )
        expect(screen.getByText('settings.typographyConnectionRequired')).toBeInTheDocument()
      })
      expect(pipelinePosts).toBe(0)
      view.unmount()
      useEditorUiStore.getState().clearError()
    }
  })

  it('shows a configuration error without starting Smart Typography when the planner engine id is missing or blank', async () => {
    let plannerEngineId: string | undefined
    let pipelinePosts = 0
    server.use(
      http.get('/api/v1/config', () =>
        HttpResponse.json({
          pipeline: {
            typography_planner: plannerEngineId,
            renderer: 'koharu-renderer',
          },
          providers: [{ id: 'openai-compatible', base_url: 'https://planner.test/v1' }],
          typography_planner: { enabled: false, model_id: 'planner-model' },
        }),
      ),
      http.post('/api/v1/pipelines', () => {
        pipelinePosts += 1
        return HttpResponse.json({ operationId: 'unexpected' })
      }),
    )

    for (const unconfiguredEngineId of [undefined, '   ']) {
      plannerEngineId = unconfiguredEngineId
      const view = renderWithQuery(
        <>
          <CanvasToolbar />
          <ActivityBubble />
        </>,
      )
      await userEvent.click(await screen.findByTestId('toolbar-typography'))

      await waitFor(() => {
        expect(useEditorUiStore.getState().error?.message).toBe(
          'settings.typographyConnectionRequired',
        )
        expect(screen.getByText('settings.typographyConnectionRequired')).toBeInTheDocument()
      })
      expect(pipelinePosts).toBe(0)
      view.unmount()
      useEditorUiStore.getState().clearError()
    }
  })

  it('shows a configuration error without starting Smart Typography when the shared Base URL is missing or blank', async () => {
    let baseUrl: string | undefined
    let pipelinePosts = 0
    server.use(
      http.get('/api/v1/config', () =>
        HttpResponse.json({
          pipeline: {
            typography_planner: 'cloud-typography-planner',
            renderer: 'koharu-renderer',
          },
          providers: [{ id: 'openai-compatible', base_url: baseUrl }],
          typography_planner: { enabled: false, model_id: 'planner-model' },
        }),
      ),
      http.post('/api/v1/pipelines', () => {
        pipelinePosts += 1
        return HttpResponse.json({ operationId: 'unexpected' })
      }),
    )

    for (const unconfiguredBaseUrl of [undefined, '   ']) {
      baseUrl = unconfiguredBaseUrl
      const view = renderWithQuery(
        <>
          <CanvasToolbar />
          <ActivityBubble />
        </>,
      )
      await userEvent.click(await screen.findByTestId('toolbar-typography'))

      await waitFor(() => {
        expect(useEditorUiStore.getState().error?.message).toBe(
          'settings.typographyConnectionRequired',
        )
        expect(screen.getByText('settings.typographyConnectionRequired')).toBeInTheDocument()
      })
      expect(pipelinePosts).toBe(0)
      view.unmount()
      useEditorUiStore.getState().clearError()
    }
  })

  it('shows the typography spinner and disables its action while a job is running', async () => {
    useJobsStore.getState().started('typography-job', 'pipeline')
    useJobsStore.getState().progress({
      jobId: 'typography-job',
      status: { status: 'running' },
      step: 'typography',
      currentPage: 0,
      totalPages: 1,
      currentStepIndex: 1,
      totalSteps: 2,
      overallPercent: 50,
    })

    renderWithQuery(<CanvasToolbar />)

    const button = await screen.findByTestId('toolbar-typography')
    expect(button).toBeDisabled()
    expect(button.querySelector('svg')).toHaveClass('animate-spin')
  })
})
