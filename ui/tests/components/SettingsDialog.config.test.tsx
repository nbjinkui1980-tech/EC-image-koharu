import { act, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { http, HttpResponse } from 'msw'
import { beforeEach, describe, expect, it } from 'vitest'

import { SettingsDialog } from '@/components/SettingsDialog'
import { getGetConfigQueryKey } from '@/lib/api'
import { queryClient } from '@/lib/queryClient'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'

import { renderWithQuery } from '../helpers'
import { server } from '../msw/server'

const BASE_CONFIG = {
  providers: [],
  data: { path: '/tmp/koharu-t02' },
  http: { connect_timeout: 10, read_timeout: 30, max_retries: 2 },
}

function useConfigHandlers() {
  server.use(
    http.get('/api/v1/config', () => HttpResponse.json(BASE_CONFIG)),
    http.get('/api/v1/llm/catalog', () =>
      HttpResponse.json({
        providers: [{ id: 'openai', name: 'OpenAI', status: 'missing_configuration' }],
      }),
    ),
    http.get('/api/v1/engines', () => HttpResponse.json({})),
    http.get('/api/v1/meta', () => HttpResponse.json({ version: 'test' })),
  )
}

async function openProviderEditor() {
  const user = userEvent.setup()
  renderWithQuery(<SettingsDialog open={true} onOpenChange={() => {}} defaultTab='providers' />)
  await user.click(await screen.findByRole('button', { name: 'OpenAI' }))
  const input = await screen.findByPlaceholderText('settings.apiKeyPlaceholderEmpty')
  return { user, input: input as HTMLInputElement }
}

describe('SettingsDialog config persistence', () => {
  beforeEach(() => {
    useEditorUiStore.setState({ error: undefined })
  })

  it('config_save_failure_keeps_draft_and_shows_error', async () => {
    useConfigHandlers()
    let patchCalls = 0
    server.use(
      http.patch('/api/v1/config', () => {
        patchCalls += 1
        return HttpResponse.json({ error: 'boom' }, { status: 500 })
      }),
    )

    const { user, input } = await openProviderEditor()
    await user.type(input, 'sk-test-key')
    await user.click(screen.getByRole('button', { name: 'settings.apiKeySave' }))

    await waitFor(() => expect(patchCalls).toBe(1))
    await waitFor(() => expect(useEditorUiStore.getState().error?.message).toBeTruthy())
    expect(input.value).toBe('sk-test-key')
  })

  it('config_save_older_response_does_not_overwrite_newer_edit', async () => {
    useConfigHandlers()
    const pending: Array<{ key: string; resolve: () => void }> = []
    server.use(
      http.patch('/api/v1/config', async ({ request }) => {
        const body = (await request.json()) as {
          providers?: Array<{ id: string; apiKey?: string }>
        }
        const key = body.providers?.[0]?.apiKey ?? ''
        return new Promise((resolve) =>
          pending.push({
            key,
            resolve: () =>
              resolve(HttpResponse.json({ providers: [{ id: 'openai', api_key: key }] })),
          }),
        )
      }),
    )

    const { user, input } = await openProviderEditor()
    await user.type(input, 'key-aaa')
    await user.click(screen.getByRole('button', { name: 'settings.apiKeySave' }))
    await waitFor(() => expect(pending.length).toBe(1))

    await user.clear(input)
    await user.type(input, 'key-bbb')
    await user.click(screen.getByRole('button', { name: 'settings.apiKeySave' }))
    await waitFor(() => expect(pending.length).toBe(2))

    pending[1]!.resolve()
    await waitFor(() => {
      const cached = queryClient.getQueryData(getGetConfigQueryKey()) as
        | { providers?: Array<{ api_key?: string }> }
        | undefined
      expect(cached?.providers?.[0]?.api_key).toBe('key-bbb')
    })
    pending[0]!.resolve()
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })

    const cached = queryClient.getQueryData(getGetConfigQueryKey()) as
      | { providers?: Array<{ api_key?: string }> }
      | undefined
    expect(cached?.providers?.[0]?.api_key).toBe('key-bbb')
  })

  it('stale_save_failure_after_newer_success_stays_silent', async () => {
    useConfigHandlers()
    let call = 0
    let releaseFirst!: () => void
    const firstGate = new Promise<void>((resolve) => {
      releaseFirst = resolve
    })
    server.use(
      http.patch('/api/v1/config', async ({ request }) => {
        const body = (await request.json()) as {
          providers?: Array<{ id: string; apiKey?: string }>
        }
        const key = body.providers?.[0]?.apiKey ?? ''
        call += 1
        if (call === 1) {
          await firstGate
          return HttpResponse.json({ error: 'late failure' }, { status: 500 })
        }
        return HttpResponse.json({ providers: [{ id: 'openai', api_key: key }] })
      }),
    )

    const { user, input } = await openProviderEditor()
    await user.type(input, 'key-aaa')
    await user.click(screen.getByRole('button', { name: 'settings.apiKeySave' }))
    await waitFor(() => expect(call).toBe(1))

    await user.clear(input)
    await user.type(input, 'key-bbb')
    await user.click(screen.getByRole('button', { name: 'settings.apiKeySave' }))
    await waitFor(() => expect(input.value).toBe(''))

    releaseFirst()
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })

    expect(useEditorUiStore.getState().error).toBeUndefined()
    const cached = queryClient.getQueryData(getGetConfigQueryKey()) as
      | { providers?: Array<{ api_key?: string }> }
      | undefined
    expect(cached?.providers?.[0]?.api_key).toBe('key-bbb')
  })
})
