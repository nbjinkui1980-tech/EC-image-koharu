import { spawnSync } from 'node:child_process'
import { realpath, readdir, rm, stat, statfs } from 'node:fs/promises'
import path from 'node:path'

import { acquireCargoPruneLease } from './cargo-target-lock'

export const GIB = 1024 ** 3

const MIN_FREE_BYTES = 20 * GIB
const MAX_TARGET_BYTES = 100 * GIB
const PRUNE_AFTER_DAYS = 30
const MAX_NEXT_BYTES = 1 * GIB
const SYSTEM_TEMP_ROOTS = ['/tmp', '/private/tmp', '/System/Volumes/Data/private/tmp']
const EXTERNAL_VOLUME_ROOT = '/Volumes/G'
const SHARED_TARGET_ROOT = '/Volumes/G/EC-image-koharu/target'

export type StorageSnapshot = {
  systemFreeBytes: number
  externalFreeBytes: number
  targetBytes: number
  nextBytes: number
}

export function storageViolations(snapshot: StorageSnapshot): string[] {
  const violations = []
  if (snapshot.systemFreeBytes < MIN_FREE_BYTES) {
    violations.push('System Data free space is below 20 GiB.')
  }
  if (snapshot.externalFreeBytes < MIN_FREE_BYTES) {
    violations.push('G volume free space is below 20 GiB.')
  }
  if (snapshot.targetBytes > MAX_TARGET_BYTES) violations.push('target exceeds 100 GiB.')
  if (snapshot.nextBytes > MAX_NEXT_BYTES) violations.push('ui/.next exceeds 1 GiB.')
  return violations
}

export function shouldPruneTarget(targetBytes: number): boolean {
  return targetBytes > MAX_TARGET_BYTES
}

export function cargoTargetViolation(
  targetRoot: string,
  sharedTargetRoot?: string,
): string | undefined {
  const resolvedTarget = path.resolve(targetRoot)
  if (
    SYSTEM_TEMP_ROOTS.some(
      (temporaryRoot) =>
        resolvedTarget === temporaryRoot ||
        resolvedTarget.startsWith(`${temporaryRoot}${path.sep}`),
    )
  ) {
    return 'CARGO_TARGET_DIR must not use macOS system temporary storage.'
  }
  if (sharedTargetRoot && resolvedTarget !== path.resolve(sharedTargetRoot)) {
    return 'CARGO_TARGET_DIR must equal KOHARU_SHARED_TARGET_DIR.'
  }
}

export function sharedTargetViolation(sharedTargetRoot?: string): string | undefined {
  if (!sharedTargetRoot) {
    return `KOHARU_SHARED_TARGET_DIR is required; set it to ${SHARED_TARGET_ROOT}.`
  }
  if (path.resolve(sharedTargetRoot) !== SHARED_TARGET_ROOT) {
    return `KOHARU_SHARED_TARGET_DIR must equal ${SHARED_TARGET_ROOT}.`
  }
}

export function isSeparateMountedFilesystem(
  volumeDevice: number,
  rootDevice: number,
  dataDevice: number,
): boolean {
  return volumeDevice !== rootDevice && volumeDevice !== dataDevice
}

async function realpathNearestExisting(root: string): Promise<{
  existingPath: string
  resolvedPath: string
}> {
  try {
    const resolvedPath = await realpath(root)
    return { existingPath: resolvedPath, resolvedPath }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
    const parent = path.dirname(root)
    if (parent === root) throw error
    const resolvedParent = await realpathNearestExisting(parent)
    return {
      existingPath: resolvedParent.existingPath,
      resolvedPath: path.join(resolvedParent.resolvedPath, path.basename(root)),
    }
  }
}

export async function resolveVerifiedSharedTarget(sharedTargetRoot?: string): Promise<string> {
  const violation = sharedTargetViolation(sharedTargetRoot)
  if (violation) throw new Error(violation)

  const target = await realpathNearestExisting(path.resolve(sharedTargetRoot!))
  const volume = await realpath(EXTERNAL_VOLUME_ROOT)
  const [targetStat, volumeStat, rootStat, dataStat] = await Promise.all([
    stat(target.existingPath),
    stat(volume),
    stat('/'),
    stat('/System/Volumes/Data'),
  ])
  if (
    volume !== EXTERNAL_VOLUME_ROOT ||
    !isSeparateMountedFilesystem(volumeStat.dev, rootStat.dev, dataStat.dev) ||
    targetStat.dev !== volumeStat.dev ||
    (target.resolvedPath !== volume && !target.resolvedPath.startsWith(`${volume}${path.sep}`))
  ) {
    throw new Error('KOHARU_SHARED_TARGET_DIR must resolve to the mounted G volume.')
  }
  return target.resolvedPath
}

export function pruneTargetViolation(
  targetRoot: string,
  sharedTargetRoot?: string,
): string | undefined {
  if (!sharedTargetRoot) {
    return 'prune-rust requires KOHARU_SHARED_TARGET_DIR to protect the shared Cargo cache.'
  }
  const targetViolation = cargoTargetViolation(targetRoot, sharedTargetRoot)
  if (targetViolation) return targetViolation

  const resolvedTarget = path.resolve(targetRoot)
  if (
    resolvedTarget !== EXTERNAL_VOLUME_ROOT &&
    !resolvedTarget.startsWith(`${EXTERNAL_VOLUME_ROOT}${path.sep}`)
  ) {
    return 'prune-rust only operates on the shared Cargo cache under /Volumes/G.'
  }
}

export function cargoSweepFailed(status: number | null, output: string): boolean {
  return status !== 0 || output.includes('[ERROR]')
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

async function statfsNearestExisting(root: string) {
  try {
    return await statfs(root)
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
    const parent = path.dirname(root)
    if (parent === root) throw error
    return await statfsNearestExisting(parent)
  }
}

async function inspectStorage(root: string, targetRoot: string): Promise<StorageSnapshot> {
  const [systemFilesystem, externalFilesystem, targetBytes, nextBytes] = await Promise.all([
    statfsNearestExisting('/System/Volumes/Data'),
    statfsNearestExisting(targetRoot),
    directorySize(targetRoot),
    directorySize(path.join(root, 'ui', '.next')),
  ])
  return {
    systemFreeBytes: Number(systemFilesystem.bavail) * Number(systemFilesystem.bsize),
    externalFreeBytes: Number(externalFilesystem.bavail) * Number(externalFilesystem.bsize),
    targetBytes,
    nextBytes,
  }
}

function formatGib(bytes: number): string {
  return `${(bytes / GIB).toFixed(1)} GiB`
}

function printSnapshot(snapshot: StorageSnapshot) {
  process.stdout.write(
    `Storage: Data ${formatGib(snapshot.systemFreeBytes)} free, G ${formatGib(snapshot.externalFreeBytes)} free, target ${formatGib(snapshot.targetBytes)}, ui/.next ${formatGib(snapshot.nextBytes)}\n`,
  )
}

function printSweepOutput(result: ReturnType<typeof spawnSync>) {
  const stdout = result.stdout?.toString() ?? ''
  const stderr = result.stderr?.toString() ?? ''
  if (stdout) process.stdout.write(stdout)
  if (stderr) process.stderr.write(stderr)
  return `${stdout}\n${stderr}`
}

function activeCargoProcessViolation(): string | undefined {
  const result = spawnSync('pgrep', ['-fl', 'cargo|rustc'], { encoding: 'utf8' })
  if (result.error) throw result.error
  if (result.status === 1) return
  if (result.status !== 0)
    throw new Error('Unable to inspect active Cargo processes before pruning.')
  return 'Rust cache pruning requires all Cargo and rustc processes to finish first.'
}

function runCargoSweep(root: string, dryRun: boolean) {
  const result = spawnSync(
    'cargo',
    ['sweep', ...(dryRun ? ['--dry-run'] : []), '--time', String(PRUNE_AFTER_DAYS), root],
    { env: process.env, encoding: 'utf8' },
  )
  if (result.error) throw result.error
  const output = printSweepOutput(result)
  if (output.includes('no such command: `sweep`')) {
    throw new Error(
      'prune-rust requires cargo-sweep 0.8.0: cargo install cargo-sweep --version 0.8.0 --locked',
    )
  }
  if (cargoSweepFailed(result.status, output)) {
    throw new Error('cargo-sweep did not complete a clean TTL-based prune.')
  }
}

async function main() {
  const root = path.resolve(import.meta.dir, '..')
  const command = process.argv[2] ?? 'check'

  if (command === 'clean-ui') {
    await rm(path.join(root, 'ui', '.next'), { recursive: true, force: true })
    process.stdout.write('Removed ui/.next.\n')
    return
  }

  if (command !== 'check' && command !== 'status' && command !== 'prune-rust') {
    throw new Error(`Unknown storage command: ${command}`)
  }

  let targetRoot = path.resolve(
    root,
    process.env.KOHARU_SHARED_TARGET_DIR ?? process.env.CARGO_TARGET_DIR ?? 'target',
  )
  if (command === 'check' || command === 'prune-rust') {
    const configuredTarget = process.env.CARGO_TARGET_DIR ?? process.env.KOHARU_SHARED_TARGET_DIR
    const violation =
      command === 'prune-rust'
        ? pruneTargetViolation(configuredTarget ?? targetRoot, process.env.KOHARU_SHARED_TARGET_DIR)
        : cargoTargetViolation(configuredTarget ?? targetRoot, process.env.KOHARU_SHARED_TARGET_DIR)
    try {
      if (violation) throw new Error(violation)
      targetRoot = await resolveVerifiedSharedTarget(process.env.KOHARU_SHARED_TARGET_DIR)
      process.env.CARGO_TARGET_DIR = targetRoot
    } catch (error) {
      process.stderr.write(`Storage guard blocked this command:\n- ${(error as Error).message}\n`)
      process.exitCode = 1
      return
    }
  }

  const snapshot = await inspectStorage(root, targetRoot)
  printSnapshot(snapshot)
  if (command === 'prune-rust') {
    if (!shouldPruneTarget(snapshot.targetBytes)) {
      process.stdout.write('Rust cache retained: shared target has not exceeded 100 GiB.\n')
      return
    }

    const activeProcessViolation = activeCargoProcessViolation()
    if (activeProcessViolation) {
      process.stderr.write(`Storage guard blocked this command:\n- ${activeProcessViolation}\n`)
      process.exitCode = 1
      return
    }

    const lease = await acquireCargoPruneLease(targetRoot)
    try {
      const activeProcessAfterLease = activeCargoProcessViolation()
      if (activeProcessAfterLease) {
        process.stderr.write(`Storage guard blocked this command:\n- ${activeProcessAfterLease}\n`)
        process.exitCode = 1
        return
      }
      runCargoSweep(root, true)
      runCargoSweep(root, false)

      const after = await inspectStorage(root, targetRoot)
      printSnapshot(after)
      if (shouldPruneTarget(after.targetBytes)) {
        process.stderr.write(
          `Rust cache remains above 100 GiB after removing artifacts unused for ${PRUNE_AFTER_DAYS} days.\n`,
        )
        process.exitCode = 1
      }
    } finally {
      await lease.release()
    }
    return
  }

  if (command === 'status') return

  const violations = storageViolations(snapshot)
  if (violations.length === 0) return

  process.stderr.write(
    `Storage guard blocked this command:\n- ${violations.join('\n- ')}\n` +
      `Use prune:rust to remove Rust artifacts unused for ${PRUNE_AFTER_DAYS} days, or clean:ui-cache, then retry.\n`,
  )
  process.exitCode = 1
}

if (import.meta.main) {
  await main()
}
