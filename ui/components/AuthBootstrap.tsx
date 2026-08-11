'use client'

import { useCallback, useEffect, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import {
  bootstrapDesktopSession,
  exchangeSession,
  isAuthenticated,
  isDesktop,
  onAuthenticationRequired,
} from '@/lib/auth'
import { connectEvents } from '@/lib/events'

type AuthState = 'pending' | 'authenticated' | 'error' | 'restart-required'

export function AuthBootstrap({ children }: { children: ReactNode }) {
  const { t } = useTranslation()
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
    const desktop = isDesktop()
    const authenticate = () => {
      setState('pending')
      setError('')
      if (!desktop) return
      bootstrapDesktopSession()
        .then(() => setState('authenticated'))
        .catch(() => setState('restart-required'))
    }
    const unsubscribe = onAuthenticationRequired(() => {
      if (desktop) {
        setState('restart-required')
        return
      }
      setState('pending')
      setError('')
    })
    if (!isAuthenticated()) authenticate()
    return unsubscribe
  }, [])

  useEffect(() => {
    if (state === 'authenticated') return connectEvents()
  }, [state])

  if (state === 'authenticated') {
    return <>{children}</>
  }

  if (state === 'restart-required') {
    return (
      <div
        role='alert'
        style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100vh' }}
      >
        Authentication expired. Restart Koharu.
      </div>
    )
  }

  if (state === 'pending' && isDesktop()) {
    return <div>{t('common.initializing')}</div>
  }

  return (
    <div
      style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100vh' }}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault()
          if (token) void attemptExchange(token)
        }}
        style={{ display: 'flex', flexDirection: 'column', gap: 8, width: 300 }}
      >
        <input
          type='password'
          placeholder='Enter authentication token'
          value={token}
          onChange={(event) => setToken(event.target.value)}
          autoFocus
        />
        <button type='submit' disabled={!token}>
          {t('updater.retry')}
        </button>
        {error && <p style={{ color: 'red' }}>{error}</p>}
      </form>
    </div>
  )
}
