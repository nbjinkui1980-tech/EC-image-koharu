import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { http, HttpResponse } from 'msw'
import { describe, expect, it, vi } from 'vitest'

import { SettingsDialog } from '@/components/SettingsDialog'
import type { AppConfig, EngineCatalog } from '@/lib/api/schemas'

import { renderWithQuery } from '../helpers'
import { server } from '../msw/server'

const engines: EngineCatalog = {
  bubbleSegmenters: [],
  detectors: [
    {
      id: 'pp-doclayout-v3',
      name: 'PP-DocLayoutV3',
      produces: ['text_boxes'],
    },
  ],
  fontDetectors: [],
  inpainters: [],
  ocr: [],
  renderers: [],
  segmenters: [],
  translators: [],
}

function config(sourceTextPolicy: 'han_only' | 'all_text'): AppConfig {
  return {
    pipeline: {
      detector: 'pp-doclayout-v3',
      source_text_policy: sourceTextPolicy,
    },
    providers: [],
    typography_planner: { enabled: false, model_id: null },
  }
}

describe('source text policy settings', () => {
  it('saves the policy without changing the selected detector and reflects server state', async () => {
    let currentConfig = config('han_only')
    const patches: unknown[] = []
    server.use(
      http.get('/api/v1/config', () => HttpResponse.json(currentConfig)),
      http.get('/api/v1/llm/catalog', () => HttpResponse.json({ localModels: [], providers: [] })),
      http.get('/api/v1/engines', () => HttpResponse.json(engines)),
      http.get('/api/v1/meta', () => HttpResponse.json({ version: 'test' })),
      http.patch('/api/v1/config', async ({ request }) => {
        const patch = (await request.json()) as {
          pipeline?: { detector?: string; sourceTextPolicy?: 'han_only' | 'all_text' }
        }
        patches.push(patch)
        currentConfig = config(patch.pipeline?.sourceTextPolicy ?? 'han_only')
        return HttpResponse.json(currentConfig)
      }),
    )

    const view = renderWithQuery(
      <SettingsDialog open onOpenChange={vi.fn()} defaultTab='engines' />,
    )

    await userEvent.click(await screen.findByTestId('source-text-policy'))
    await userEvent.click(await screen.findByRole('option', { name: 'settings.sourceTextAll' }))
    await waitFor(() => {
      const patch = patches.at(-1) as {
        pipeline?: { detector?: string; sourceTextPolicy?: string }
      }
      expect(patch.pipeline?.sourceTextPolicy).toBe('all_text')
      expect(patch.pipeline?.detector).toBe('pp-doclayout-v3')
    })

    currentConfig = config('han_only')
    view.unmount()
    renderWithQuery(<SettingsDialog open onOpenChange={vi.fn()} defaultTab='engines' />)

    expect(await screen.findByTestId('source-text-policy')).toHaveTextContent(
      'settings.sourceTextChinese',
    )
  })
})
