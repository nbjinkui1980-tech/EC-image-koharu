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

function trimTerminalJobs(jobs: Record<string, JobEntry>): Record<string, JobEntry> {
  const entries = Object.entries(jobs)
  let terminal = 0
  for (const [, job] of entries) {
    if (job.status !== 'running') terminal += 1
  }
  if (terminal <= MAX_TERMINAL_JOBS) return jobs
  const next = { ...jobs }
  let evict = terminal - MAX_TERMINAL_JOBS
  for (const [id, job] of entries) {
    if (evict === 0) break
    if (job.status !== 'running') {
      delete next[id]
      evict -= 1
    }
  }
  return next
}

type JobsState = {
  jobs: Record<string, JobEntry>
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
  setSnapshot: (jobs) => {
    const next: Record<string, JobEntry> = {}
    for (const job of jobs) next[job.id] = job
    set({ jobs: next })
  },
  started: (id, kind) =>
    set((s) => ({
      jobs: { ...s.jobs, [id]: { id, kind, status: 'running' } },
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
      }
    }),
  finished: (id, status, error) =>
    set((s) => {
      const existing = s.jobs[id] ?? { id, kind: 'pipeline', status }
      return {
        jobs: trimTerminalJobs({ ...s.jobs, [id]: { ...existing, status, error: error ?? null } }),
      }
    }),
  clear: () => set({ jobs: {} }),
  byStatus: (status) => Object.values(get().jobs).filter((j) => j.status === status),
}))
