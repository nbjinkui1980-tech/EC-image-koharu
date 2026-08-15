'use client'

export const isTauri = (): boolean =>
  typeof window !== 'undefined' && !!(window as any).__TAURI_INTERNALS__

export async function openExternalUrl(url: string): Promise<void> {
  if (isTauri()) {
    const { openUrl } = await import('@tauri-apps/plugin-opener')
    await openUrl(url)
    return
  }

  if (typeof window !== 'undefined') {
    window.open(url, '_blank', 'noopener,noreferrer')
  }
}

const VERIFICATION_URL_HOSTS = new Set(['auth.openai.com'])

export async function openVerificationUrl(url: string): Promise<void> {
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    throw new Error('verification url is not a valid URL')
  }
  if (parsed.protocol !== 'https:') {
    throw new Error('verification url must use https')
  }
  if (parsed.username || parsed.password) {
    throw new Error('verification url must not embed credentials')
  }
  if (!VERIFICATION_URL_HOSTS.has(parsed.host)) {
    throw new Error('verification url host is not approved')
  }
  await openExternalUrl(url)
}
