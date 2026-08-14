import { beforeEach, describe, expect, it, vi } from 'vitest'

import { filenameFromContentDisposition, saveBlob } from '@/lib/io/saveBlob'

describe('filenameFromContentDisposition', () => {
  it('returns undefined for null / empty', () => {
    expect(filenameFromContentDisposition(null)).toBeUndefined()
    expect(filenameFromContentDisposition('')).toBeUndefined()
  })

  it('parses RFC5987 filename*', () => {
    expect(filenameFromContentDisposition("attachment; filename*=UTF-8''my%20file.zip")).toBe(
      'my file.zip',
    )
  })

  it('parses quoted filename', () => {
    expect(filenameFromContentDisposition('attachment; filename="report.psd"')).toBe('report.psd')
  })

  it('parses unquoted filename', () => {
    expect(filenameFromContentDisposition('attachment; filename=report.psd')).toBe('report.psd')
  })

  it('prefers filename* when both are present', () => {
    const header = 'attachment; filename="ascii.zip"; filename*=UTF-8\'\'unicode.zip'
    expect(filenameFromContentDisposition(header)).toBe('unicode.zip')
  })

  it('returns undefined when no filename is found', () => {
    expect(filenameFromContentDisposition('attachment')).toBeUndefined()
  })
})

// ---------------------------------------------------------------------------
// Zip extraction safety (Tauri path)
// ---------------------------------------------------------------------------

import { zipSync } from 'fflate'

const { isTauriMock, openDialog, writeFileMock, mkdirMock } = vi.hoisted(() => ({
  isTauriMock: vi.fn(),
  openDialog: vi.fn(),
  writeFileMock: vi.fn().mockResolvedValue(undefined),
  mkdirMock: vi.fn().mockResolvedValue(undefined),
}))

vi.mock('@/lib/backend', () => ({ isTauri: isTauriMock }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: openDialog, save: vi.fn() }))
vi.mock('@tauri-apps/plugin-fs', () => ({
  writeFile: writeFileMock,
  mkdir: mkdirMock,
}))

function zipBlob(entries: Record<string, Uint8Array>): Blob {
  const bytes = zipSync(entries)
  return new Blob([bytes.buffer as ArrayBuffer], { type: 'application/zip' })
}

// AR08-T01 RED: a zip entry must be validated before any write — traversal,
// absolute, drive, UNC, and backslash variants can currently write outside
// the folder the user picked.
describe('zip entry path validation (Tauri extract)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    isTauriMock.mockReturnValue(true)
    openDialog.mockResolvedValue('/chosen/folder')
  })

  it('rejects parent-traversal entries without any write', async () => {
    const blob = zipBlob({ '../evil.png': new Uint8Array([1]) })
    await expect(saveBlob(blob, 'export.zip')).rejects.toThrow()
    expect(writeFileMock).not.toHaveBeenCalled()
    expect(mkdirMock).not.toHaveBeenCalled()
  })

  it('rejects absolute, drive, UNC, and backslash-traversal entries', async () => {
    for (const name of ['/abs.png', 'C:/win.png', '//unc/x.png', 'nested\\..\\evil.png']) {
      vi.clearAllMocks()
      isTauriMock.mockReturnValue(true)
      openDialog.mockResolvedValue('/chosen/folder')
      const blob = zipBlob({ [name]: new Uint8Array([1]) })
      await expect(saveBlob(blob, 'export.zip')).rejects.toThrow()
      expect(writeFileMock).not.toHaveBeenCalled()
    }
  })

  it('rejects dot and empty path segments; directory entries are skipped', async () => {
    for (const name of ['a/./b.png', 'a//b.png']) {
      vi.clearAllMocks()
      isTauriMock.mockReturnValue(true)
      openDialog.mockResolvedValue('/chosen/folder')
      const blob = zipBlob({ [name]: new Uint8Array([1]) })
      await expect(saveBlob(blob, 'export.zip')).rejects.toThrow()
      expect(writeFileMock).not.toHaveBeenCalled()
    }
  })

  it('rejects a mixed zip with zero partial writes', async () => {
    const blob = zipBlob({
      'good-1.png': new Uint8Array([1]),
      'good-2.png': new Uint8Array([2]),
      '../evil.png': new Uint8Array([3]),
    })
    await expect(saveBlob(blob, 'export.zip')).rejects.toThrow()
    expect(writeFileMock).not.toHaveBeenCalled()
    expect(mkdirMock).not.toHaveBeenCalled()
  })

  // Lock: plain nested entries still extract under the chosen folder.
  it('extracts plain nested entries under the chosen folder', async () => {
    const blob = zipBlob({
      'dir/page.png': new Uint8Array([1]),
      'page.png': new Uint8Array([2]),
    })
    await expect(saveBlob(blob, 'export.zip')).resolves.toBe(true)
    const written = writeFileMock.mock.calls.map((call) => call[0])
    expect(written).toContain('/chosen/folder/dir/page.png')
    expect(written).toContain('/chosen/folder/page.png')
  })
})
