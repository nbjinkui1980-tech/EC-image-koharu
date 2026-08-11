import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

const updater = vi.hoisted(() => vi.fn(({ children }: { children: React.ReactNode }) => children))

vi.mock('@/components/AuthBootstrap', () => ({
  AuthBootstrap: () => <div>authentication-pending</div>,
}))
vi.mock('@/components/Updater', () => ({ UpdaterProvider: updater }))

import { Providers } from '@/app/providers'

describe('Providers authentication gate', () => {
  it('does not mount the updater or children before authentication', async () => {
    render(<Providers>business-ui</Providers>)

    await screen.findByText('authentication-pending')
    expect(screen.queryByText('business-ui')).not.toBeInTheDocument()
    expect(updater).not.toHaveBeenCalled()
  })
})
