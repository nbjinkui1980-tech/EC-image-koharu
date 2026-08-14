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

export async function saveBlob(blob: Blob, defaultName: string): Promise<boolean> {
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
      const { unzipSync } = await import('fflate')
      const entries = unzipSync(new Uint8Array(await blob.arrayBuffer()))
      // Validate every entry before any mkdir/write: one pure
      // validation/normalization boundary, and a rejected archive leaves
      // zero partial files behind.
      const plan: { path: string; bytes: Uint8Array }[] = []
      for (const [name, bytes] of Object.entries(entries)) {
        if (name.replace(/\\/g, '/').endsWith('/')) continue // directory entry
        const safe = sanitizeZipEntryName(name)
        if (safe === null) throw new Error(`unsafe zip entry: ${name}`)
        plan.push({ path: `${folder}/${safe}`, bytes })
      }
      for (const { path, bytes } of plan) {
        const slash = path.lastIndexOf('/')
        if (slash > folder.length) {
          const dir = path.substring(0, slash)
          await mkdir(dir, { recursive: true }).catch(() => {})
        }
        await writeFile(path, bytes)
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
