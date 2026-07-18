import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { http, HttpResponse } from 'msw'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { TypographyPlannerSettings } from '@/components/settings/TypographyPlannerSettings'
import { SettingsDialog } from '@/components/SettingsDialog'
import { getGetConfigQueryKey } from '@/lib/api'
import type { AppConfig, LlmCatalog, LlmProviderCatalog } from '@/lib/api/schemas'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'

import { renderWithQuery } from '../helpers'
import { server } from '../msw/server'

const plannerCatalog: LlmProviderCatalog = {
  id: 'openai-compatible',
  name: 'OpenAI-compatible',
  requiresBaseUrl: true,
  requiresApiKey: false,
  hasApiKey: false,
  baseUrl: 'https://models.example/v1',
  status: 'ready',
  error: null,
  models: [
    {
      name: 'Planner B',
      languages: [],
      target: {
        kind: 'provider',
        providerId: 'openai-compatible',
        modelId: 'planner-b',
      },
    },
  ],
}

function config(overrides: Partial<AppConfig> = {}): AppConfig {
  return {
    providers: [{ id: 'openai-compatible', base_url: 'https://models.example/v1' }],
    typography_planner: { enabled: false, model_id: null },
    ...overrides,
  }
}

function catalog(providers: LlmProviderCatalog[] = [plannerCatalog]): LlmCatalog {
  return { localModels: [], providers }
}

const emptyEngines = {
  bubbleSegmenters: [],
  detectors: [],
  fontDetectors: [],
  inpainters: [],
  ocr: [],
  renderers: [],
  segmenters: [],
  translators: [],
}

describe('TypographyPlannerSettings', () => {
  beforeEach(() => {
    useEditorUiStore.setState({
      selectedTarget: {
        kind: 'provider',
        providerId: 'openai-compatible',
        modelId: 'translator-a',
      },
    })
  })

  it('uses the shared OpenAI-compatible catalog without a second provider selector', async () => {
    const onChange = vi.fn()
    const onOpenProviders = vi.fn()
    const view = renderWithQuery(
      <TypographyPlannerSettings
        config={config()}
        catalog={plannerCatalog}
        onChange={onChange}
        onOpenProviders={onOpenProviders}
      />,
    )

    expect(screen.getAllByRole('combobox')).toHaveLength(1)
    expect(screen.queryByText('settings.localLlmPreset')).not.toBeInTheDocument()
    expect(screen.getByText('settings.typographyReloadTranslation')).toBeInTheDocument()

    await userEvent.click(screen.getByTestId('typography-planner-model'))
    await userEvent.click(await screen.findByRole('option', { name: 'Planner B' }))
    expect(onChange).toHaveBeenLastCalledWith({ enabled: false, model_id: 'planner-b' })

    view.rerender(
      <TypographyPlannerSettings
        config={config({ typography_planner: { enabled: false, model_id: 'planner-b' } })}
        catalog={plannerCatalog}
        onChange={onChange}
        onOpenProviders={onOpenProviders}
      />,
    )
    await userEvent.click(screen.getByTestId('typography-planner-enabled'))
    expect(onChange).toHaveBeenLastCalledWith({ enabled: true, model_id: 'planner-b' })
  })

  it('allows disabling automatic typography when the shared connection becomes unavailable', async () => {
    const onOpenProviders = vi.fn()
    const onChange = vi.fn()
    renderWithQuery(
      <TypographyPlannerSettings
        config={config({
          providers: [{ id: 'openai-compatible', base_url: null }],
          typography_planner: { enabled: true, model_id: 'missing-model' },
        })}
        catalog={{ ...plannerCatalog, baseUrl: null, models: [], status: 'missing_configuration' }}
        onChange={onChange}
        onOpenProviders={onOpenProviders}
      />,
    )

    const enabled = screen.getByTestId('typography-planner-enabled')
    expect(enabled).toBeEnabled()
    await userEvent.click(enabled)
    expect(onChange).toHaveBeenLastCalledWith({ enabled: false, model_id: 'missing-model' })
    expect(screen.getByTestId('typography-planner-invalid-model')).toHaveTextContent(
      'missing-model',
    )
    expect(screen.getByText('settings.typographyConnectionRequired')).toBeInTheDocument()
    await userEvent.click(screen.getByTestId('typography-provider-settings'))
    expect(onOpenProviders).toHaveBeenCalledOnce()
  })

  it('saves only planner fields, updates the config cache, and does not reload the catalog', async () => {
    let currentConfig = config()
    let catalogRequests = 0
    const patches: unknown[] = []
    let currentLlmPuts = 0
    server.use(
      http.get('/api/v1/config', () => HttpResponse.json(currentConfig)),
      http.get('/api/v1/llm/catalog', () => {
        catalogRequests += 1
        return HttpResponse.json(catalog())
      }),
      http.get('/api/v1/engines', () => HttpResponse.json(emptyEngines)),
      http.get('/api/v1/meta', () => HttpResponse.json({ version: 'test' })),
      http.patch('/api/v1/config', async ({ request }) => {
        const patch = await request.json()
        patches.push(patch)
        const typographyPlanner = (
          patch as { typographyPlanner: { enabled?: boolean; modelId?: string | null } }
        ).typographyPlanner
        currentConfig = {
          ...currentConfig,
          typography_planner: {
            enabled: typographyPlanner.enabled ?? false,
            model_id: typographyPlanner.modelId ?? null,
          },
        }
        return HttpResponse.json(currentConfig)
      }),
      http.put('/api/v1/llm/current', () => {
        currentLlmPuts += 1
        return HttpResponse.json({ status: 'ready' })
      }),
    )

    const { client } = renderWithQuery(
      <SettingsDialog open onOpenChange={vi.fn()} defaultTab='typography' />,
    )

    await userEvent.click(await screen.findByTestId('typography-planner-model'))
    await userEvent.click(await screen.findByRole('option', { name: 'Planner B' }))
    await waitFor(() =>
      expect(patches).toEqual([{ typographyPlanner: { enabled: false, modelId: 'planner-b' } }]),
    )

    await userEvent.click(screen.getByTestId('typography-planner-enabled'))
    await waitFor(() =>
      expect(patches).toEqual([
        { typographyPlanner: { enabled: false, modelId: 'planner-b' } },
        { typographyPlanner: { enabled: true, modelId: 'planner-b' } },
      ]),
    )

    expect(catalogRequests).toBe(1)
    expect(currentLlmPuts).toBe(0)
    expect(useEditorUiStore.getState().selectedTarget).toEqual({
      kind: 'provider',
      providerId: 'openai-compatible',
      modelId: 'translator-a',
    })
    expect(client.getQueryData(getGetConfigQueryKey())).toEqual(currentConfig)
  })
})
