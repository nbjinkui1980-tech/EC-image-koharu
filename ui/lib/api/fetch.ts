export class ApiError extends Error {
  readonly status: number
  readonly body: unknown
  constructor(status: number, message: string, body?: unknown) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.body = body
  }
}

export const fetchWithAuth = async (url: string, options?: RequestInit): Promise<Response> => {
  const res = await fetch(url, { ...options, credentials: 'same-origin' })
  if (res.status === 401) {
    const { notifyAuthenticationRequired } = await import('@/lib/auth')
    notifyAuthenticationRequired()
  }
  return res
}

export const fetchApi = async <T>(url: string, options?: RequestInit): Promise<T> => {
  const res = await fetchWithAuth(url, options)
  if (!res.ok) {
    const body = await res.json().catch(() => null)
    const message =
      (body && typeof body === 'object' && 'message' in body && typeof body.message === 'string'
        ? body.message
        : null) ??
      res.statusText ??
      `HTTP ${res.status}`
    throw new ApiError(res.status, message, body)
  }
  if ([204, 205, 304].includes(res.status)) {
    return undefined as T
  }
  const contentType = res.headers.get('content-type') ?? ''
  if (!contentType.includes('json')) {
    return (await res.blob()) as T
  }
  return res.json()
}

/**
 * orval per-operation mutator for binary downloads that must keep response
 * headers (e.g. Content-Disposition). Same error contract as fetchApi; the
 * body is returned as a Blob alongside the raw headers.
 */
export type FullResponse = { blob: Blob; headers: Headers }

export const fetchApiFullResponse = async <T>(url: string, options?: RequestInit): Promise<T> => {
  const res = await fetchWithAuth(url, options)
  if (!res.ok) {
    const body = await res.json().catch(() => null)
    const message =
      (body && typeof body === 'object' && 'message' in body && typeof body.message === 'string'
        ? body.message
        : null) ??
      res.statusText ??
      `HTTP ${res.status}`
    throw new ApiError(res.status, message, body)
  }
  const blob = await res.blob()
  return { blob, headers: res.headers } as T
}
