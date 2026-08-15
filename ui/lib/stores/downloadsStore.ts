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

const MAX_TERMINAL_DOWNLOADS = 256

function trimTerminalDownloads(
  downloads: Record<string, DownloadProgress>,
): Record<string, DownloadProgress> {
  const entries = Object.entries(downloads)
  let terminal = 0
  for (const [, download] of entries) {
    if (download.status.status !== 'downloading') terminal += 1
  }
  if (terminal <= MAX_TERMINAL_DOWNLOADS) return downloads
  const next = { ...downloads }
  let evict = terminal - MAX_TERMINAL_DOWNLOADS
  for (const [id, download] of entries) {
    if (evict === 0) break
    if (download.status.status !== 'downloading') {
      delete next[id]
      evict -= 1
    }
  }
  return next
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
      downloads: trimTerminalDownloads({ ...s.downloads, [p.id]: p }),
    })),
  remove: (id) =>
    set((s) => {
      const { [id]: _removed, ...downloads } = s.downloads
      return { downloads }
    }),
  clear: () => set({ downloads: {} }),
  byStatus: (status) => Object.values(get().downloads).filter((d) => d.status.status === status),
}))
