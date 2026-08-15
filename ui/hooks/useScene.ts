'use client'

import { useGetSceneJson } from '@/lib/api'
import { ApiError } from '@/lib/api/fetch'
import type { Scene } from '@/lib/api/schemas'

/**
 * Backend is the source of truth for the scene. Components read it through
 * this hook — which is just a thin wrapper around the orval-generated
 * `useGetSceneJson` query. Mutations must invalidate `getGetSceneJsonQueryKey`
 * for the UI to pick up changes (see `lib/io/scene.ts`).
 *
 * When no project is open, `GET /scene.json` returns 400; React Query stores
 * that as an error and `scene` is `null`.
 */
export function useScene(): { scene: Scene | null; epoch: number } {
  const { data, error, isError } = useGetSceneJson({
    query: {
      retry: false,
      staleTime: Infinity,
      gcTime: Infinity,
      structuralSharing: true,
    },
  })
  // React Query preserves `data` across a failed refetch. Only an explicit
  // 400 "no project open" clears the scene (project closed); transient
  // failures (5xx/network) keep the last good scene, and the next
  // invalidation refetches.
  if (isError && error instanceof ApiError && error.status === 400) {
    return { scene: null, epoch: 0 }
  }
  return {
    scene: data?.scene ?? null,
    epoch: data?.epoch ?? 0,
  }
}
