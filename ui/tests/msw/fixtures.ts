export function projectSummary(
  id: string,
  name: string,
  updatedAtMs = 0,
): { id: string; name: string; path: string; updatedAtMs: number } {
  return { id, name, path: `/tmp/${id}`, updatedAtMs }
}

export const readyLlmState = {
  status: 'ready',
  target: null,
  error: null,
} as const
