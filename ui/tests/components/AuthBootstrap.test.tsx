import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { AuthBootstrap } from '@/components/AuthBootstrap'

const mocks = vi.hoisted(() => ({
  bootstrapDesktopSession: vi.fn<() => Promise<void>>(),
  exchangeSession: vi.fn<(credential: string) => Promise<void>>(),
  connectEvents: vi.fn(() => vi.fn()),
  authRequired: undefined as (() => void) | undefined,
  desktop: true,
  authenticated: false,
}))

vi.mock('@/lib/auth', () => ({
  bootstrapDesktopSession: mocks.bootstrapDesktopSession,
  exchangeSession: mocks.exchangeSession,
  isAuthenticated: () => mocks.authenticated,
  isDesktop: () => mocks.desktop,
  onAuthenticationRequired: (listener: () => void) => {
    mocks.authRequired = listener
    return vi.fn()
  },
}))

vi.mock('@/lib/events', () => ({ connectEvents: mocks.connectEvents }))

describe('AuthBootstrap', () => {
  beforeEach(() => {
    mocks.bootstrapDesktopSession.mockReset().mockResolvedValue()
    mocks.exchangeSession.mockReset().mockResolvedValue()
    mocks.connectEvents.mockClear()
    mocks.authRequired = undefined
    mocks.desktop = true
    mocks.authenticated = false
  })

  it('shows restart-required without requesting a second desktop proof', async () => {
    render(<AuthBootstrap>ready</AuthBootstrap>)

    await screen.findByText('ready')
    expect(mocks.bootstrapDesktopSession).toHaveBeenCalledTimes(1)

    act(() => mocks.authRequired?.())

    await screen.findByRole('alert')
    expect(screen.getByText('Authentication expired. Restart Koharu.')).toBeInTheDocument()
    expect(screen.queryByText('ready')).not.toBeInTheDocument()
    expect(mocks.bootstrapDesktopSession).toHaveBeenCalledTimes(1)
  })

  it('mounts neither children nor SSE before desktop authentication succeeds', async () => {
    let resolveBootstrap: (() => void) | undefined
    mocks.bootstrapDesktopSession.mockReturnValue(
      new Promise<void>((resolve) => {
        resolveBootstrap = resolve
      }),
    )

    render(<AuthBootstrap>ready</AuthBootstrap>)

    expect(screen.queryByText('ready')).not.toBeInTheDocument()
    expect(mocks.connectEvents).not.toHaveBeenCalled()

    await act(async () => resolveBootstrap?.())
    await screen.findByText('ready')
    expect(mocks.connectEvents).toHaveBeenCalledTimes(1)
  })

  it('returns headless clients to token entry after a runtime 401', async () => {
    mocks.desktop = false
    render(<AuthBootstrap>ready</AuthBootstrap>)

    const input = screen.getByPlaceholderText('Enter authentication token')
    fireEvent.change(input, { target: { value: 'headless-token' } })
    fireEvent.submit(input.closest('form')!)

    await screen.findByText('ready')
    expect(mocks.exchangeSession).toHaveBeenCalledWith('headless-token')
    expect(mocks.connectEvents).toHaveBeenCalledTimes(1)

    act(() => mocks.authRequired?.())

    await waitFor(() =>
      expect(screen.getByPlaceholderText('Enter authentication token')).toBeInTheDocument(),
    )
    expect(screen.queryByText('ready')).not.toBeInTheDocument()
  })

  it('shows restart-required without mounting children or SSE when desktop bootstrap rejects', async () => {
    mocks.bootstrapDesktopSession.mockRejectedValue(new Error('IPC failed'))

    render(<AuthBootstrap>ready</AuthBootstrap>)

    await screen.findByRole('alert')
    expect(screen.getByText('Authentication expired. Restart Koharu.')).toBeInTheDocument()
    expect(screen.queryByText('ready')).not.toBeInTheDocument()
    expect(mocks.connectEvents).not.toHaveBeenCalled()
  })

  it('keeps headless client at token form with error after rejection, then authenticates on retry', async () => {
    mocks.desktop = false
    mocks.exchangeSession.mockRejectedValueOnce(new Error('Bad token'))
    render(<AuthBootstrap>ready</AuthBootstrap>)

    const input = screen.getByPlaceholderText('Enter authentication token')
    fireEvent.change(input, { target: { value: 'bad' } })
    fireEvent.submit(input.closest('form')!)

    await waitFor(() => expect(screen.getByText('Authentication failed')).toBeInTheDocument())
    expect(screen.queryByText('ready')).not.toBeInTheDocument()

    mocks.exchangeSession.mockResolvedValueOnce()
    fireEvent.change(input, { target: { value: 'good' } })
    fireEvent.submit(input.closest('form')!)

    await screen.findByText('ready')
    expect(mocks.exchangeSession).toHaveBeenCalledTimes(2)
    expect(mocks.connectEvents).toHaveBeenCalledTimes(1)
  })

  it.each([true, false])(
    'keeps an authenticated client mounted without bootstrapping again (desktop=%s)',
    async (desktop) => {
      mocks.desktop = desktop
      mocks.authenticated = true

      render(<AuthBootstrap>ready</AuthBootstrap>)

      await screen.findByText('ready')
      expect(mocks.bootstrapDesktopSession).not.toHaveBeenCalled()
      expect(mocks.exchangeSession).not.toHaveBeenCalled()
      expect(mocks.connectEvents).toHaveBeenCalledTimes(1)
    },
  )
})
