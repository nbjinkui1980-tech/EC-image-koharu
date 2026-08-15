'use client'

import { create } from 'zustand'

import type { DownloadProgress, DownloadStatus } from '@/lib/api/schemas'

type DownloadsState = {
  downloads: Record<string, DownloadProgress>
  order: string[]
  setSnapshot: (downloads: DownloadProgress[]) => void
  progress: (p: DownloadProgress) => void
  remove: (id: string) => void
  clear: () => void
  byStatus: (status: DownloadStatus['status']) => DownloadProgress[]
}

const MAX_TERMINAL_DOWNLOADS = 256

function trimTerminalDownloads(
  downloads: Record<string, DownloadProgress>,
  order: string[],
): { downloads: Record<string, DownloadProgress>; order: string[] } {
  const terminalIds = order.filter(
    (id) => downloads[id] && downloads[id].status.status !== 'downloading',
  )
  if (terminalIds.length <= MAX_TERMINAL_DOWNLOADS) return { downloads, order }
  const evict = new Set(terminalIds.slice(0, terminalIds.length - MAX_TERMINAL_DOWNLOADS))
  const nextDownloads = { ...downloads }
  for (const id of evict) delete nextDownloads[id]
  return { downloads: nextDownloads, order: order.filter((id) => !evict.has(id)) }
}

export const useDownloadsStore = create<DownloadsState>()((set, get) => ({
  downloads: {},
  order: [],
  setSnapshot: (downloads) => {
    const next: Record<string, DownloadProgress> = {}
    for (const download of downloads) next[download.id] = download
    set(
      trimTerminalDownloads(
        next,
        downloads.map((d) => d.id),
      ),
    )
  },
  progress: (p) =>
    set((s) => {
      const downloads = { ...s.downloads, [p.id]: p }
      const order = s.downloads[p.id] ? s.order : [...s.order, p.id]
      return trimTerminalDownloads(downloads, order)
    }),
  remove: (id) =>
    set((s) => {
      const { [id]: _removed, ...downloads } = s.downloads
      return { downloads, order: s.order.filter((existing) => existing !== id) }
    }),
  clear: () => set({ downloads: {}, order: [] }),
  byStatus: (status) => Object.values(get().downloads).filter((d) => d.status.status === status),
}))
