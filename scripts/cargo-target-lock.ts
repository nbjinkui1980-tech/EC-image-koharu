import { randomUUID } from 'node:crypto'
import { mkdir, readFile, readdir, rename, rm, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'

const LOCK_ROOT = '.koharu-target-lock'
const GATE = 'gate'
const BUILDS = 'builds'
const PRUNING = 'pruning'
const GATE_RETRIES = 600
const GATE_RETRY_MS = 10
const BUILD_RETRY_MS = 100
const OWNER_WRITE_GRACE_MS = 5_000
const RECOVERING = '.recovering-'

type LeaseOwner = {
  pid: number
  token: string
}

export type CargoTargetLease = {
  release: () => Promise<void>
}

export const __cargoTargetLockTestHooks: {
  afterDeadLeaseOwnerRead?: (directory: string) => Promise<void>
  onBuildBlocked?: (directory: string) => void
  onRecoveryBlocked?: (directory: string) => void
} = {}

function lockPaths(targetRoot: string) {
  const root = path.join(targetRoot, LOCK_ROOT)
  return {
    builds: path.join(root, BUILDS),
    gate: path.join(root, GATE),
    pruning: path.join(root, PRUNING),
  }
}

function isAlreadyExists(error: unknown) {
  return (error as NodeJS.ErrnoException).code === 'EEXIST'
}

function isMissing(error: unknown) {
  return (error as NodeJS.ErrnoException).code === 'ENOENT'
}

async function waitForGate() {
  await new Promise((resolve) => setTimeout(resolve, GATE_RETRY_MS))
}

async function waitForBuild() {
  await new Promise((resolve) => setTimeout(resolve, BUILD_RETRY_MS))
}

function processIsAlive(pid: number) {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ESRCH') return false
    return true
  }
}

async function readOwner(ownerPath: string): Promise<LeaseOwner | undefined> {
  try {
    const value = JSON.parse(await readFile(ownerPath, 'utf8')) as Partial<LeaseOwner>
    if (Number.isInteger(value.pid) && value.pid! > 0 && typeof value.token === 'string') {
      return { pid: value.pid!, token: value.token }
    }
  } catch (error) {
    if (!isMissing(error) && !(error instanceof SyntaxError)) throw error
  }
}

async function writeOwner(directory: string, token: string) {
  await writeFile(
    path.join(directory, 'owner'),
    JSON.stringify({ pid: process.pid, token } satisfies LeaseOwner),
  )
}

function recoveryPrefix(directory: string) {
  return `${path.basename(directory)}${RECOVERING}`
}

async function hasActiveRecovery(directory: string): Promise<boolean> {
  const parent = path.dirname(directory)
  let entries: string[]
  try {
    entries = await readdir(parent)
  } catch (error) {
    if (isMissing(error)) return false
    throw error
  }

  for (const entry of entries) {
    if (!entry.startsWith(recoveryPrefix(directory))) continue
    const marker = path.join(parent, entry)
    const owner = await readOwner(path.join(marker, 'owner'))
    if (owner && processIsAlive(owner.pid)) return true
    if (!owner) {
      try {
        if (Date.now() - (await stat(marker)).mtimeMs < OWNER_WRITE_GRACE_MS) return true
      } catch (error) {
        if (isMissing(error)) continue
        throw error
      }
    }
    await rm(marker, { recursive: true, force: true })
  }
  return false
}

async function removeDeadLease(directory: string): Promise<boolean> {
  const recoveryToken = randomUUID()
  const recovery = `${directory}${RECOVERING}${recoveryToken}`
  await mkdir(recovery)
  try {
    await writeOwner(recovery, recoveryToken)
    const owner = await readOwner(path.join(directory, 'owner'))
    await __cargoTargetLockTestHooks.afterDeadLeaseOwnerRead?.(directory)
    if (owner) {
      if (processIsAlive(owner.pid)) return false
    } else {
      try {
        const directoryStat = await stat(directory)
        if (Date.now() - directoryStat.mtimeMs < OWNER_WRITE_GRACE_MS) return false
      } catch (error) {
        if (isMissing(error)) return true
        throw error
      }
    }

    const quarantine = `${directory}.stale-${randomUUID()}`
    try {
      await rename(directory, quarantine)
    } catch (error) {
      if (isMissing(error)) return true
      throw error
    }

    const movedOwner = await readOwner(path.join(quarantine, 'owner'))
    const movedLeaseChanged = owner
      ? movedOwner?.token !== owner.token
      : movedOwner !== undefined && processIsAlive(movedOwner.pid)
    if (movedLeaseChanged) {
      try {
        await rename(quarantine, directory)
      } catch (error) {
        if (!isAlreadyExists(error)) throw error
      }
      return false
    }

    await rm(quarantine, { recursive: true, force: true })
    return true
  } finally {
    await rm(recovery, { recursive: true, force: true })
  }
}

async function releaseOwnedDirectory(directory: string, token: string) {
  const owner = await readOwner(path.join(directory, 'owner'))
  if (owner?.token === token) await rm(directory, { recursive: true, force: true })
}

async function withGate<T>(targetRoot: string, action: () => Promise<T>): Promise<T> {
  const { gate } = lockPaths(targetRoot)
  await mkdir(path.dirname(gate), { recursive: true })
  const token = randomUUID()

  for (let attempt = 0; attempt < GATE_RETRIES; attempt += 1) {
    if (await hasActiveRecovery(gate)) {
      __cargoTargetLockTestHooks.onRecoveryBlocked?.(gate)
      await waitForGate()
      continue
    }

    try {
      await mkdir(gate)
    } catch (error) {
      if (!isAlreadyExists(error)) throw error
      if (await removeDeadLease(gate)) continue
      await waitForGate()
      continue
    }

    try {
      await writeOwner(gate, token)
      if (await hasActiveRecovery(gate)) {
        __cargoTargetLockTestHooks.onRecoveryBlocked?.(gate)
        await waitForGate()
        continue
      }
      return await action()
    } finally {
      await releaseOwnedDirectory(gate, token)
    }
  }

  throw new Error(
    'Rust target coordination is busy; retry after the current lock operation finishes.',
  )
}

async function activeBuildMarkers(builds: string): Promise<string[]> {
  try {
    const markers = await readdir(builds)
    const active: string[] = []
    for (const marker of markers) {
      const markerPath = path.join(builds, marker)
      const owner = await readOwner(markerPath)
      if (!owner || processIsAlive(owner.pid)) active.push(marker)
      else await rm(markerPath, { force: true })
    }
    return active
  } catch (error) {
    if (isMissing(error)) return []
    throw error
  }
}

export async function acquireCargoBuildLease(targetRoot: string): Promise<CargoTargetLease> {
  const { builds, pruning } = lockPaths(targetRoot)
  let marker = ''

  while (!marker) {
    await withGate(targetRoot, async () => {
      await removeDeadLease(pruning)
      try {
        await readdir(pruning)
        throw new Error('Rust cache pruning is in progress; retry the build after it finishes.')
      } catch (error) {
        if (!isMissing(error)) throw error
      }

      await mkdir(builds, { recursive: true })
      if ((await activeBuildMarkers(builds)).length > 0) {
        __cargoTargetLockTestHooks.onBuildBlocked?.(builds)
        return
      }

      const token = randomUUID()
      marker = path.join(builds, `${process.pid}-${token}`)
      await writeFile(marker, JSON.stringify({ pid: process.pid, token } satisfies LeaseOwner))
    })
    if (!marker) await waitForBuild()
  }

  return {
    release: async () => {
      await withGate(targetRoot, async () => {
        await rm(marker, { force: true })
      })
    },
  }
}

export async function acquireCargoPruneLease(targetRoot: string): Promise<CargoTargetLease> {
  const { builds, pruning } = lockPaths(targetRoot)
  const token = randomUUID()

  await withGate(targetRoot, async () => {
    await removeDeadLease(pruning)
    try {
      await readdir(pruning)
      throw new Error('Rust cache pruning is already in progress; retry after it finishes.')
    } catch (error) {
      if (!isMissing(error)) throw error
    }

    const activeBuilds = await activeBuildMarkers(builds)
    if (activeBuilds.length > 0) {
      throw new Error('Rust cache pruning requires all shared-target builds to finish first.')
    }

    await mkdir(pruning)
    await writeOwner(pruning, token)
  })

  return {
    release: async () => {
      await withGate(targetRoot, async () => {
        await releaseOwnedDirectory(pruning, token)
      })
    },
  }
}
