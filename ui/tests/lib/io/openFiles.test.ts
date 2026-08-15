import { beforeEach, describe, expect, it, vi } from 'vitest'

const { openDialog, readFileMock, readDirMock, fileOpenMock, directoryOpenMock, isTauriMock } =
  vi.hoisted(() => ({
    openDialog: vi.fn(),
    readFileMock: vi.fn(),
    readDirMock: vi.fn(),
    fileOpenMock: vi.fn(),
    directoryOpenMock: vi.fn(),
    isTauriMock: vi.fn(),
  }))

vi.mock('@/lib/backend', () => ({ isTauri: isTauriMock }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: openDialog }))
vi.mock('@tauri-apps/plugin-fs', () => ({
  readFile: readFileMock,
  readDir: readDirMock,
}))
vi.mock('browser-fs-access', () => ({
  fileOpen: fileOpenMock,
  directoryOpen: directoryOpenMock,
}))

import { openImageFiles, openImageFolder } from '@/lib/io/openFiles'

// AR05-T05A RED: on Tauri the image pickers must return File[] (read via the
// plugin-fs temp scope) exactly like the web path, so all downstream upload
// code is platform-neutral multipart. Today the Tauri branches hand back raw
// paths instead, so every assertion below fails until GREEN.
describe('openImageFiles on Tauri', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    isTauriMock.mockReturnValue(true)
  })

  it('returns picked images as File[] read via the fs scope', async () => {
    openDialog.mockResolvedValue(['/pics/b.png', '/pics/a.jpg'])
    readFileMock.mockImplementation(async (path: string) =>
      path.endsWith('.png') ? new Uint8Array([1, 2]) : new Uint8Array([3]),
    )

    const result = (await openImageFiles()) as unknown as File[]

    expect(Array.isArray(result)).toBe(true)
    expect(result).toHaveLength(2)
    expect(result[0]).toBeInstanceOf(File)
    expect(result[0].name).toBe('b.png')
    expect(result[0].type).toBe('image/png')
    expect(result[1].name).toBe('a.jpg')
    expect(result[1].type).toBe('image/jpeg')
  })

  it('returns [] without touching the fs when the dialog is cancelled', async () => {
    openDialog.mockResolvedValue(null)

    const result = await openImageFiles()

    expect(result).toEqual([])
    expect(readFileMock).not.toHaveBeenCalled()
  })
})

describe('openImageFolder on Tauri', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    isTauriMock.mockReturnValue(true)
  })

  it('returns image entries as File[] sorted by name, skipping non-images', async () => {
    openDialog.mockResolvedValue('/pics')
    readDirMock.mockResolvedValue([
      { isFile: true, name: 'b.png' },
      { isFile: true, name: 'notes.txt' },
      { isFile: true, name: 'a.webp' },
      { isFile: false, name: 'nested' },
    ])
    readFileMock.mockResolvedValue(new Uint8Array([9]))

    const result = (await openImageFolder()) as unknown as File[]

    expect(result.map((f) => f.name)).toEqual(['a.webp', 'b.png'])
    expect(result.every((f) => f instanceof File)).toBe(true)
    expect(readFileMock).toHaveBeenCalledTimes(2)
  })
})

describe('image pickers on the web', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    isTauriMock.mockReturnValue(false)
  })

  it('openImageFiles returns the picked File[] unchanged', async () => {
    const file = new File([new Uint8Array([1])], 'x.png', { type: 'image/png' })
    fileOpenMock.mockResolvedValue([file])

    const result = (await openImageFiles()) as unknown as File[]

    expect(result).toEqual([file])
  })

  it('openImageFiles returns [] on abort', async () => {
    fileOpenMock.mockRejectedValue(Object.assign(new Error('cancelled'), { name: 'AbortError' }))

    const result = await openImageFiles()

    expect(result).toEqual([])
  })
})
