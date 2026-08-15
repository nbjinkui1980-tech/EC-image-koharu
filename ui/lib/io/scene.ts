'use client'

import {
  applyCommand,
  createPages,
  createProject,
  deleteCurrentProject,
  exportCurrentProject,
  getConfig,
  getGetConfigQueryKey,
  getGetCurrentLlmQueryKey,
  getGetSceneJsonQueryKey,
  importProject,
  patchConfig,
  putCurrentProject,
  redo,
  reorderTextNodes,
  startPipeline,
  undo,
} from '@/lib/api'
import type { FullResponse } from '@/lib/api/fetch'
import type {
  ConfigPatch,
  CreateProjectRequest,
  ExportProjectRequest,
  Op,
  OpenProjectRequest,
  ProjectSummary,
  ReadingOrder,
  Scene,
  SceneSnapshot,
} from '@/lib/api/schemas'
import { filenameFromContentDisposition } from '@/lib/io/saveBlob'
import { queryClient } from '@/lib/queryClient'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'
import { usePreferencesStore } from '@/lib/stores/preferencesStore'
import { useSelectionStore } from '@/lib/stores/selectionStore'

/**
 * Imperative action helpers. Every mutation below is a thin wrapper that
 *   1. calls the orval-generated request function (never raw `fetch`), and
 *   2. invalidates the React Query cache entries affected by the change.
 *
 * The UI reads scene / config / llm state via the generated `useGet*` hooks;
 * after each mutation React Query refetches — no client-side scene reducer,
 * no optimistic mirroring, backend is the single source of truth.
 */

export const invalidateScene = () =>
  queryClient.invalidateQueries({ queryKey: getGetSceneJsonQueryKey() })

const invalidateConfig = () => queryClient.invalidateQueries({ queryKey: getGetConfigQueryKey() })

const invalidateLlm = () => queryClient.invalidateQueries({ queryKey: getGetCurrentLlmQueryKey() })

// Ops ------------------------------------------------------------------------

let historyMutationQueue: Promise<void> = Promise.resolve()

const enqueueHistoryMutation = (run: () => Promise<void>): Promise<void> => {
  const next = historyMutationQueue.then(run, run)
  historyMutationQueue = next.catch(() => undefined)
  return next
}

export async function applyOp(opOrBuild: Op | ((scene: Scene) => Op | null)): Promise<void> {
  await enqueueHistoryMutation(async () => {
    let op: Op | null
    if (typeof opOrBuild === 'function') {
      const latest = queryClient.getQueryData(getGetSceneJsonQueryKey()) as
        | SceneSnapshot
        | undefined
      op = latest ? opOrBuild(latest.scene) : null
      if (!op) return
    } else {
      op = opOrBuild
    }
    await applyCommand(op)
    await invalidateScene()
  })
}

export async function undoOp(): Promise<void> {
  await enqueueHistoryMutation(async () => {
    await undo()
    await invalidateScene()
  })
}

export async function redoOp(): Promise<void> {
  await enqueueHistoryMutation(async () => {
    await redo()
    await invalidateScene()
  })
}

export async function reorderPageTextNodes(pageId: string, order: ReadingOrder): Promise<void> {
  await reorderTextNodes(pageId, order)
  await invalidateScene()
}

// Auto-render ---------------------------------------------------------------
//
// `queueAutoRender(pageId)` schedules a debounced renderer-pipeline invocation
// so a text-block edit (move/resize/translation/color/etc.) produces an
// updated rendered image without the user running Render manually.
//
// Coalescing is essential: slider drags and typing emit many ops per second;
// the trailing-edge debounce fires one render after the edits settle.

const AUTO_RENDER_DEBOUNCE_MS = 500

const autoRenderTimers = new Map<string, ReturnType<typeof setTimeout>>()

function cancelQueuedAutoRender(pageId?: string): void {
  if (pageId === undefined) {
    for (const timer of autoRenderTimers.values()) clearTimeout(timer)
    autoRenderTimers.clear()
    return
  }
  const timer = autoRenderTimers.get(pageId)
  if (timer) clearTimeout(timer)
  autoRenderTimers.delete(pageId)
}

export function queueAutoRender(pageId: string): void {
  cancelQueuedAutoRender(pageId)
  autoRenderTimers.set(
    pageId,
    setTimeout(() => {
      autoRenderTimers.delete(pageId)
      void runAutoRenderWithFeedback(pageId)
    }, AUTO_RENDER_DEBOUNCE_MS),
  )
}

export async function runAutoRenderNow(pageId: string): Promise<void> {
  cancelQueuedAutoRender(pageId)
  await runAutoRenderWithFeedback(pageId)
}

async function runAutoRenderWithFeedback(pageId: string): Promise<void> {
  try {
    await runAutoRender(pageId)
  } catch (err) {
    console.error('Auto-render failed:', err)
    useEditorUiStore.getState().showError(String(err))
  }
}

async function runAutoRender(pageId: string): Promise<void> {
  const cfg = await getConfig()
  const renderer = cfg.pipeline?.renderer
  if (!renderer) return
  const defaultFont = usePreferencesStore.getState().defaultFont
  const targetLanguage = useEditorUiStore.getState().selectedLanguage
  await startPipeline({ steps: [renderer], pages: [pageId], defaultFont, targetLanguage })
}

/** Select every text node on the active page. No-op if no project/page open. */
export function selectAllTextNodesOnCurrentPage(): void {
  const pageId = useSelectionStore.getState().pageId
  if (!pageId) return
  const snap = queryClient.getQueryData<SceneSnapshot>(getGetSceneJsonQueryKey())
  const page = snap?.scene?.pages?.[pageId]
  if (!page) return
  const ids: string[] = []
  for (const [id, node] of Object.entries(page.nodes)) {
    if (node && 'text' in node.kind) ids.push(id)
  }
  useSelectionStore.getState().selectMany(ids)
}

// Project lifecycle ----------------------------------------------------------

export async function createAndOpenProject(req: CreateProjectRequest): Promise<ProjectSummary> {
  const summary = await createProject(req)
  await invalidateScene()
  return summary
}

export async function switchProject(req: OpenProjectRequest): Promise<void> {
  cancelQueuedAutoRender()
  await putCurrentProject(req)
  await invalidateScene()
}

export async function closeProject(): Promise<void> {
  cancelQueuedAutoRender()
  await deleteCurrentProject()
  await invalidateScene()
}

// Pages import ---------------------------------------------------------------

export async function uploadPages(files: File[], replace: boolean): Promise<string[]> {
  const form = new FormData()
  for (const file of files) form.append('file', file, file.name)
  form.append('replace', replace ? 'true' : 'false')
  const res = await createPages({ body: form })
  await invalidateScene()
  return res.pages
}

export async function uploadKhrArchive(file: File): Promise<ProjectSummary> {
  const bytes = await file.arrayBuffer()
  const summary = await importProject({
    body: bytes,
    headers: { 'Content-Type': 'application/zip' },
  })
  await invalidateScene()
  return summary
}

// Export ---------------------------------------------------------------------

/**
 * Export wrapper that keeps the server-supplied filename.
 *
 * The backend returns the raw file for single-page exports (e.g. a PNG or
 * PSD with `Content-Type: image/png`), and a zip when the format produces
 * multiple files. The raw-file shortcut means we can't hardcode `.zip` in
 * the UI — we'd end up feeding a PNG to `unzipSync` and crashing. Read
 * the `Content-Disposition` filename so the caller gets the correct
 * extension + `blob.type` to drive the save path.
 */
export async function exportProject(
  req: ExportProjectRequest,
): Promise<{ blob: Blob; filename?: string }> {
  // The generated signature says Blob per the spec, but the configured
  // fetchApiFullResponse mutator returns the full response — headers carry
  // the server's Content-Disposition filename.
  const { blob, headers } = (await exportCurrentProject(req)) as unknown as FullResponse
  const filename = filenameFromContentDisposition(headers.get('content-disposition'))
  return { blob, filename }
}

// Config ---------------------------------------------------------------------

export async function updateConfig(patch: ConfigPatch): Promise<void> {
  await patchConfig(patch)
  await invalidateConfig()
}

// LLM ------------------------------------------------------------------------

export function invalidateCurrentLlm(): Promise<void> {
  return invalidateLlm()
}
