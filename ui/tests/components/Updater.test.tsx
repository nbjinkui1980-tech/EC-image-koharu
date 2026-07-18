import { act, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { UpdaterProvider } from '@/components/Updater'

const updater = vi.hoisted(() => ({
  check: vi.fn(),
}))

vi.mock('@tauri-apps/plugin-updater', () => updater)

describe('UpdaterProvider', () => {
  beforeEach(() => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', {
      configurable: true,
      value: {},
    })
    updater.check.mockResolvedValue(null)
  })

  afterEach(() => {
    delete (window as typeof window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
    vi.unstubAllEnvs()
  })

  it('does not contact the release updater from a development desktop build', async () => {
    vi.stubEnv('NODE_ENV', 'development')

    render(
      <UpdaterProvider>
        <div>content</div>
      </UpdaterProvider>,
    )

    await act(async () => {
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(updater.check).not.toHaveBeenCalled()
  })
})
