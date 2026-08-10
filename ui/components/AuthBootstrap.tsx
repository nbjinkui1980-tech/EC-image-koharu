'use client'

import { useCallback, useEffect, useState, type ReactNode } from 'react'

import {
  bootstrapDesktopSession,
  exchangeSession,
  isAuthenticated,
  isDesktop,
  onAuthenticationRequired,
} from '@/lib/auth'

type AuthState = 'pending' | 'authenticated' | 'error'

export function AuthBootstrap({ children }: { children: ReactNode }) {
  const [state, setState] = useState<AuthState>(() =>
    isAuthenticated() ? 'authenticated' : 'pending',
  )
  const [token, setToken] = useState('')
  const [error, setError] = useState('')

  const attemptExchange = useCallback(async (credential: string) => {
    try {
      await exchangeSession(credential)
      setState('authenticated')
    } catch {
      setError('Authentication failed')
      setState('error')
    }
  }, [])

  useEffect(() => {
    if (isDesktop()) {
      bootstrapDesktopSession()
        .then(() => setState('authenticated'))
        .catch(() => setState('error'))
    }
    return onAuthenticationRequired(() => {
      setState('pending')
      setError('')
    })
  }, [])

  if (state === 'authenticated') {
    return <>{children}</>
  }

  if (state === 'pending' && isDesktop()) {
    return null
  }

  return (
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100vh' }}>
      <form
        onSubmit={(e) => {
          e.preventDefault()
          if (token) attemptExchange(token)
        }}
        style={{ display: 'flex', flexDirection: 'column', gap: 8, width: 300 }}
      >
        <input
          type='password'
          placeholder='Enter authentication token'
          value={token}
          onChange={(e) => setToken(e.target.value)}
          autoFocus
        />
        <button type='submit' disabled={!token}>
          Authenticate
        </button>
        {error && <p style={{ color: 'red' }}>{error}</p>}
      </form>
    </div>
  )
}
