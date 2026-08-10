const AUTH_EVENT = 'koharu:auth-required'

const emitter = new EventTarget()

let authenticated = false

export function exchangeSession(credential: string): Promise<void> {
  return fetch('/api/v1/auth/session', {
    method: 'POST',
    headers: { Authorization: `Bearer ${credential}` },
  }).then((res) => {
    if (!res.ok) throw new Error(`auth exchange failed: ${res.status}`)
    authenticated = true
  })
}

export async function bootstrapDesktopSession(): Promise<void> {
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    const proof = (await invoke('desktop_bootstrap_proof')) as string
    return exchangeSession(proof)
  } catch {
    notifyAuthenticationRequired()
  }
}

export function notifyAuthenticationRequired(): void {
  authenticated = false
  emitter.dispatchEvent(new Event(AUTH_EVENT))
}

export function onAuthenticationRequired(listener: () => void): () => void {
  const handler = () => listener()
  emitter.addEventListener(AUTH_EVENT, handler)
  return () => emitter.removeEventListener(AUTH_EVENT, handler)
}

export function isAuthenticated(): boolean {
  return authenticated
}

export function isDesktop(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}
