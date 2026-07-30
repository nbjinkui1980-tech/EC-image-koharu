'use client'

import { create } from 'zustand'

import type { DownloadProgress, DownloadStatus } from '@/lib/api/schemas'

type DownloadsState = {
  downloads: Record<string, DownloadProgress>
  setSnapshot: (downloads: DownloadProgress[]) => void
  progress: (p: DownloadProgress) => void
  remove: (id: string) => void
  clear: () => void
  byStatus: (status: DownloadStatus['status']) => DownloadProgress[]
}

export const useDownloadsStore = create<DownloadsState>()((set, get) => ({
  downloads: {},
  setSnapshot: (downloads) => {
    const next: Record<string, DownloadProgress> = {}
    for (const download of downloads) next[download.id] = download
    set({ downloads: next })
  },
  progress: (p) =>
    set((s) => ({
      downloads: { ...s.downloads, [p.id]: p },
    })),
  remove: (id) =>
    set((s) => {
      const { [id]: _removed, ...downloads } = s.downloads
      return { downloads }
    }),
  clear: () => set({ downloads: {} }),
  byStatus: (status) => Object.values(get().downloads).filter((d) => d.status.status === status),
}))
