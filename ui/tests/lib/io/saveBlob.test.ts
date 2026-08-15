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

// AR08-T02 RED: every entry and the total budget must be validated before
// the first write; over-budget or invalid archives leave zero files behind.
describe('zip extraction budget (Tauri extract)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    isTauriMock.mockReturnValue(true)
    openDialog.mockResolvedValue('/chosen/folder')
  })

  it('rejects over-budget total bytes before any write', async () => {
    const budget = { maxEntries: 100, maxTotalBytes: 8, maxFileBytes: 1024 }
    const blob = zipBlob({
      'a.png': new Uint8Array([1, 2, 3, 4, 5]),
      'b.png': new Uint8Array([6, 7, 8, 9]),
    })
    await expect(saveBlob(blob, 'export.zip', budget)).rejects.toThrow(/budget/)
    expect(writeFileMock).not.toHaveBeenCalled()
    expect(mkdirMock).not.toHaveBeenCalled()
  })

  it('rejects an entry count over budget', async () => {
    const budget = { maxEntries: 1, maxTotalBytes: 1024, maxFileBytes: 1024 }
    const blob = zipBlob({
      'a.png': new Uint8Array([1]),
      'b.png': new Uint8Array([2]),
    })
    await expect(saveBlob(blob, 'export.zip', budget)).rejects.toThrow(/budget/)
    expect(writeFileMock).not.toHaveBeenCalled()
  })

  it('rejects a single file over budget via actual decompressed count', async () => {
    const budget = { maxEntries: 100, maxTotalBytes: 4096, maxFileBytes: 4 }
    const blob = zipBlob({ 'big.png': new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]) })
    await expect(saveBlob(blob, 'export.zip', budget)).rejects.toThrow(/budget/)
    expect(writeFileMock).not.toHaveBeenCalled()
  })

  // Lock: within budget everything still extracts.
  it('extracts within budget', async () => {
    const budget = { maxEntries: 2, maxTotalBytes: 1024, maxFileBytes: 1024 }
    const blob = zipBlob({
      'a.png': new Uint8Array([1]),
      'b.png': new Uint8Array([2]),
    })
    await expect(saveBlob(blob, 'export.zip', budget)).resolves.toBe(true)
    expect(writeFileMock).toHaveBeenCalledTimes(2)
  })
})
