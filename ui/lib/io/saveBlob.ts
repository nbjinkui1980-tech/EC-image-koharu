'use client'

/**
 * Cross-platform blob save.
 *
 * - **Tauri**: native save dialog (for single files) or folder dialog + unzip
 *   (for multi-file `application/zip` blobs). The server always returns a zip
 *   when a format produces multiple files; on Tauri we extract into the
 *   chosen folder so users get individual files, not a zip they have to
 *   unpack.
 *
 * - **Web**: `browser-fs-access` handles File System Access API + the legacy
 *   `<a download>` fallback. Zips are saved as-is (user unzips if desired).
 *
 * Returns `true` if the save completed, `false` if the user cancelled.
 */

import { isTauri } from '@/lib/backend'

/**
 * Normalize a zip entry name to a safe relative path, or return null when the
 * entry would escape the chosen folder: parent/dot/empty segments, POSIX
 * absolute paths, drive letters, and UNC paths (after backslash folding) are
 * all rejected. Directory entries (trailing slash) are handled by the caller.
 */
export function sanitizeZipEntryName(name: string): string | null {
  const normalized = name.replace(/\\/g, '/')
  if (normalized.startsWith('/')) return null
  if (/^[A-Za-z]:/.test(normalized)) return null
  const segments = normalized.split('/')
  if (segments.some((segment) => segment === '' || segment === '.' || segment === '..')) {
    return null
  }
  return segments.join('/')
}

/**
 * Extraction budget for zip saves (Tauri path only — the web path saves the
 * archive as-is). Entries past the budget or an archive whose declared or
 * actual decompressed size exceeds it are rejected before anything is
 * written.
 */
export interface ZipBudget {
  maxEntries: number
  maxTotalBytes: number
  maxFileBytes: number
}

export const DEFAULT_ZIP_BUDGET: ZipBudget = {
  maxEntries: 4096,
  maxTotalBytes: 4 * 1024 * 1024 * 1024,
  maxFileBytes: 2 * 1024 * 1024 * 1024,
}

function joinChunks(chunks: Uint8Array[], total: number): Uint8Array {
  if (chunks.length === 1) return chunks[0]
  const out = new Uint8Array(total)
  let offset = 0
  for (const chunk of chunks) {
    out.set(chunk, offset)
    offset += chunk.length
  }
  return out
}

/**
 * Validate and plan a zip extraction against the budget. Streams the archive
 * with fflate's Unzip: every entry name is sanitized and sizes are accounted
 * (declared first, actual decompressed bytes as the backstop) before any
 * entry content is collected — so nothing past the budget is ever allocated
 * into `plan`, and nothing is written by the caller on throw.
 */
async function planZipExtraction(
  bytes: Uint8Array,
  budget: ZipBudget,
): Promise<{ name: string; data: Uint8Array }[]> {
  const { Unzip, UnzipInflate } = await import('fflate')
  const plan: { name: string; data: Uint8Array }[] = []
  let entryCount = 0
  let totalDeclared = 0
  let failure: Error | null = null

  const unzipper = new Unzip()
  unzipper.register(UnzipInflate)
  unzipper.onfile = (file) => {
    if (failure) return
    const normalized = file.name.replace(/\\/g, '/')
    if (normalized.endsWith('/')) return // directory entry: never started, never read
    const safe = sanitizeZipEntryName(normalized)
    if (safe === null) {
      failure = new Error(`unsafe zip entry: ${file.name}`)
      return
    }
    entryCount += 1
    if (entryCount > budget.maxEntries) {
      failure = new Error(`zip budget exceeded: more than ${budget.maxEntries} entries`)
      return
    }
    const declared = file.size ?? 0
    if (declared > budget.maxFileBytes || totalDeclared + declared > budget.maxTotalBytes) {
      failure = new Error('zip budget exceeded: declared bytes over limit')
      return
    }
    totalDeclared += declared
    const declaredBefore = totalDeclared - declared
    const chunks: Uint8Array[] = []
    let actual = 0
    file.ondata = (err, chunk, final) => {
      if (failure) return
      if (err) {
        failure = err instanceof Error ? err : new Error(String(err))
        return
      }
      actual += chunk.length
      if (actual > budget.maxFileBytes || declaredBefore + actual > budget.maxTotalBytes) {
        failure = new Error('zip budget exceeded: decompressed bytes over limit')
        return
      }
      chunks.push(chunk)
      if (final) plan.push({ name: safe, data: joinChunks(chunks, actual) })
    }
    file.start()
  }
  unzipper.push(bytes, true)
  if (failure) throw failure
  return plan
}

export async function saveBlob(
  blob: Blob,
  defaultName: string,
  budget: ZipBudget = DEFAULT_ZIP_BUDGET,
): Promise<boolean> {
  // Zip detection must come from the actual content type — a single-file
  // export (PNG/PSD/khr) whose filename happens to end in `.zip` would
  // otherwise be fed to `unzipSync` and throw.
  const isZip = blob.type === 'application/zip'

  if (isTauri()) {
    const { open, save } = await import('@tauri-apps/plugin-dialog')
    const { writeFile, mkdir } = await import('@tauri-apps/plugin-fs')

    if (isZip) {
      const folder = await open({ directory: true, multiple: false })
      if (!folder || typeof folder !== 'string') return false
      const bytes = new Uint8Array(await blob.arrayBuffer())
      // Every entry and the total budget are validated before the first
      // mkdir/write — a rejected archive leaves zero partial files.
      const plan = await planZipExtraction(bytes, budget)
      for (const { name, data } of plan) {
        const full = `${folder}/${name}`
        const slash = full.lastIndexOf('/')
        if (slash > folder.length) {
          const dir = full.substring(0, slash)
          await mkdir(dir, { recursive: true }).catch(() => {})
        }
        await writeFile(full, data)
      }
      return true
    }

    const path = await save({ defaultPath: defaultName })
    if (!path || typeof path !== 'string') return false
    await writeFile(path, new Uint8Array(await blob.arrayBuffer()))
    return true
  }

  const { fileSave } = await import('browser-fs-access')
  await fileSave(blob, { fileName: defaultName })
  return true
}

/**
 * Parse a `Content-Disposition: attachment; filename="..."` header. Returns
 * the filename (or `undefined` if the header is missing/unparseable).
 */
export function filenameFromContentDisposition(header: string | null): string | undefined {
  if (!header) return undefined
  const m =
    header.match(/filename\*=UTF-8''([^;]+)/i) ??
    header.match(/filename="([^"]+)"/i) ??
    header.match(/filename=([^;]+)/i)
  if (!m) return undefined
  try {
    return decodeURIComponent(m[1].trim())
  } catch {
    return m[1].trim()
  }
}
