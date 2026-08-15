'use client'

import { create } from 'zustand'

import type { JobSummary, JobWarningEvent, PipelineProgress } from '@/lib/api/schemas'

/**
 * Live job registry, fed by SSE. Keyed by id. `progress` is attached when
 * the backend streams `JobProgress` for a running pipeline job. `warnings`
 * accumulates non-fatal step failures as they arrive; the pipeline keeps
 * running past them.
 */
export type JobEntry = JobSummary & {
  progress?: PipelineProgress
  warnings?: JobWarningEvent[]
}

const MAX_TERMINAL_JOBS = 256

function trimTerminalJobs(
  jobs: Record<string, JobEntry>,
  order: string[],
): { jobs: Record<string, JobEntry>; order: string[] } {
  const terminalIds = order.filter((id) => jobs[id] && jobs[id].status !== 'running')
  if (terminalIds.length <= MAX_TERMINAL_JOBS) return { jobs, order }
  const evict = new Set(terminalIds.slice(0, terminalIds.length - MAX_TERMINAL_JOBS))
  const nextJobs = { ...jobs }
  for (const id of evict) delete nextJobs[id]
  return { jobs: nextJobs, order: order.filter((id) => !evict.has(id)) }
}

type JobsState = {
  jobs: Record<string, JobEntry>
  order: string[]
  setSnapshot: (jobs: JobSummary[]) => void
  started: (id: string, kind: string) => void
  progress: (p: PipelineProgress) => void
  warning: (w: JobWarningEvent) => void
  finished: (id: string, status: JobSummary['status'], error: string | null | undefined) => void
  clear: () => void
  byStatus: (status: JobSummary['status']) => JobEntry[]
}

export const useJobsStore = create<JobsState>()((set, get) => ({
  jobs: {},
  order: [],
  setSnapshot: (jobs) => {
    const next: Record<string, JobEntry> = {}
    for (const job of jobs) next[job.id] = job
    set(
      trimTerminalJobs(
        next,
        jobs.map((j) => j.id),
      ),
    )
  },
  started: (id, kind) =>
    set((s) => ({
      jobs: { ...s.jobs, [id]: { id, kind, status: 'running' } },
      order: s.jobs[id] ? s.order : [...s.order, id],
    })),
  progress: (p) =>
    set((s) => {
      const existing = s.jobs[p.jobId] ?? {
        id: p.jobId,
        kind: 'pipeline',
        status: 'running' as JobSummary['status'],
      }
      return {
        jobs: { ...s.jobs, [p.jobId]: { ...existing, progress: p } },
        order: s.jobs[p.jobId] ? s.order : [...s.order, p.jobId],
      }
    }),
  warning: (w) =>
    set((s) => {
      const existing = s.jobs[w.jobId] ?? {
        id: w.jobId,
        kind: 'pipeline',
        status: 'running' as JobSummary['status'],
      }
      const warnings = existing.warnings ?? []
      return {
        jobs: { ...s.jobs, [w.jobId]: { ...existing, warnings: [...warnings, w] } },
        order: s.jobs[w.jobId] ? s.order : [...s.order, w.jobId],
      }
    }),
  finished: (id, status, error) =>
    set((s) => {
      const existing = s.jobs[id] ?? { id, kind: 'pipeline', status }
      const jobs = { ...s.jobs, [id]: { ...existing, status, error: error ?? null } }
      const order = s.jobs[id] ? s.order : [...s.order, id]
      return trimTerminalJobs(jobs, order)
    }),
  clear: () => set({ jobs: {}, order: [] }),
  byStatus: (status) => Object.values(get().jobs).filter((j) => j.status === status),
}))
