import { readdir, rm, stat, statfs } from 'node:fs/promises'
import path from 'node:path'

export const GIB = 1024 ** 3

const MIN_FREE_BYTES = 20 * GIB
const MAX_TARGET_BYTES = 16 * GIB
const MAX_NEXT_BYTES = 1 * GIB

export type StorageSnapshot = {
  freeBytes: number
  targetBytes: number
  nextBytes: number
}

export function storageViolations(snapshot: StorageSnapshot): string[] {
  const violations = []
  if (snapshot.freeBytes < MIN_FREE_BYTES) violations.push('Available disk space is below 20 GiB.')
  if (snapshot.targetBytes > MAX_TARGET_BYTES) violations.push('target exceeds 16 GiB.')
  if (snapshot.nextBytes > MAX_NEXT_BYTES) violations.push('ui/.next exceeds 1 GiB.')
  return violations
}

async function directorySize(root: string): Promise<number> {
  const directories = [root]
  const seenFiles = new Set<string>()
  let total = 0

  while (directories.length > 0) {
    const directory = directories.pop()!
    let entries
    try {
      entries = await readdir(directory, { withFileTypes: true })
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') continue
      throw error
    }

    const files: string[] = []
    for (const entry of entries) {
      const entryPath = path.join(directory, entry.name)
      if (entry.isDirectory()) directories.push(entryPath)
      else if (entry.isFile()) files.push(entryPath)
    }

    for (let offset = 0; offset < files.length; offset += 128) {
      const stats = await Promise.all(
        files.slice(offset, offset + 128).map(async (file) => {
          try {
            return await stat(file)
          } catch (error) {
            if ((error as NodeJS.ErrnoException).code === 'ENOENT') return undefined
            throw error
          }
        }),
      )
      for (const file of stats) {
        if (!file) continue
        const key = file.ino === 0 ? undefined : `${file.dev}:${file.ino}`
        if (key && seenFiles.has(key)) continue
        if (key) seenFiles.add(key)
        total += file.size
      }
    }
  }

  return total
}

async function inspectStorage(root: string): Promise<StorageSnapshot> {
  const targetRoot = path.resolve(root, process.env.CARGO_TARGET_DIR ?? 'target')
  const [filesystem, targetBytes, nextBytes] = await Promise.all([
    statfs(root),
    directorySize(targetRoot),
    directorySize(path.join(root, 'ui', '.next')),
  ])
  return {
    freeBytes: Number(filesystem.bavail) * Number(filesystem.bsize),
    targetBytes,
    nextBytes,
  }
}

function formatGib(bytes: number): string {
  return `${(bytes / GIB).toFixed(1)} GiB`
}

function printSnapshot(snapshot: StorageSnapshot) {
  process.stdout.write(
    `Storage: ${formatGib(snapshot.freeBytes)} free, target ${formatGib(snapshot.targetBytes)}, ui/.next ${formatGib(snapshot.nextBytes)}\n`,
  )
}

async function main() {
  const root = path.resolve(import.meta.dir, '..')
  const command = process.argv[2] ?? 'check'

  if (command === 'clean-ui') {
    await rm(path.join(root, 'ui', '.next'), { recursive: true, force: true })
    process.stdout.write('Removed ui/.next.\n')
    return
  }

  if (command === 'check' && process.env.CI) {
    process.stdout.write('Storage guard skipped in CI.\n')
    return
  }

  if (command !== 'check' && command !== 'status') {
    throw new Error(`Unknown storage command: ${command}`)
  }

  const snapshot = await inspectStorage(root)
  printSnapshot(snapshot)
  if (command === 'status') return

  const violations = storageViolations(snapshot)
  if (violations.length === 0) return

  process.stderr.write(
    `Storage guard blocked this command:\n- ${violations.join('\n- ')}\n` +
      'Use clean:rust:dev, clean:rust:release, clean:rust:desktop, or clean:ui-cache, then retry.\n',
  )
  process.exitCode = 1
}

if (import.meta.main) {
  await main()
}
