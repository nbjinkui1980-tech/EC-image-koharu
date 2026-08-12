import { describe, expect, test } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'

import {
  __cargoTargetLockTestHooks,
  acquireCargoBuildLease,
  acquireCargoPruneLease,
} from './cargo-target-lock'

function deferred() {
  let resolve!: () => void
  const promise = new Promise<void>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

async function withTarget(action: (targetRoot: string) => Promise<void>) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'koharu-target-lock-'))
  try {
    await action(root)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
}

describe('shared Cargo target coordination', () => {
  test('does not prune while a wrapped build is active', async () => {
    await withTarget(async (targetRoot) => {
      const build = await acquireCargoBuildLease(targetRoot)
      await expect(acquireCargoPruneLease(targetRoot)).rejects.toThrow(
        'requires all shared-target builds to finish',
      )
      await build.release()

      const prune = await acquireCargoPruneLease(targetRoot)
      await prune.release()
    })
  })

  test('does not start a wrapped build while pruning', async () => {
    await withTarget(async (targetRoot) => {
      const prune = await acquireCargoPruneLease(targetRoot)
      await expect(acquireCargoBuildLease(targetRoot)).rejects.toThrow('pruning is in progress')
      await prune.release()

      const build = await acquireCargoBuildLease(targetRoot)
      await build.release()
    })
  })

  test('recovers build, prune, and gate leases whose owner process has exited', async () => {
    await withTarget(async (targetRoot) => {
      const lockRoot = path.join(targetRoot, '.koharu-target-lock')
      const deadOwner = JSON.stringify({ pid: 2_147_483_647, token: 'stale' })

      await mkdir(path.join(lockRoot, 'builds'), { recursive: true })
      await writeFile(path.join(lockRoot, 'builds', '2147483647-stale'), deadOwner)
      const prune = await acquireCargoPruneLease(targetRoot)
      await prune.release()

      await mkdir(path.join(lockRoot, 'pruning'), { recursive: true })
      await writeFile(path.join(lockRoot, 'pruning', 'owner'), deadOwner)
      const buildAfterPrune = await acquireCargoBuildLease(targetRoot)
      await buildAfterPrune.release()

      await mkdir(path.join(lockRoot, 'gate'), { recursive: true })
      await writeFile(path.join(lockRoot, 'gate', 'owner'), deadOwner)
      const buildAfterGate = await acquireCargoBuildLease(targetRoot)
      await buildAfterGate.release()
    })
  })

  test('fences reacquisition while a stale gate owner is being recovered', async () => {
    await withTarget(async (targetRoot) => {
      const gate = path.join(targetRoot, '.koharu-target-lock', 'gate')
      await mkdir(gate, { recursive: true })
      await writeFile(
        path.join(gate, 'owner'),
        JSON.stringify({ pid: 2_147_483_647, token: 'stale' }),
      )

      const staleOwnerRead = deferred()
      const resumeRecovery = deferred()
      const reacquisitionBlocked = deferred()
      let pauseFirstRecovery = true
      __cargoTargetLockTestHooks.afterDeadLeaseOwnerRead = async (directory) => {
        if (directory !== gate || !pauseFirstRecovery) return
        pauseFirstRecovery = false
        staleOwnerRead.resolve()
        await resumeRecovery.promise
      }
      __cargoTargetLockTestHooks.onRecoveryBlocked = (directory) => {
        if (directory === gate) reacquisitionBlocked.resolve()
      }

      try {
        const first = acquireCargoBuildLease(targetRoot)
        await staleOwnerRead.promise
        let secondAcquired = false
        const second = acquireCargoBuildLease(targetRoot).then((lease) => {
          secondAcquired = true
          return lease
        })
        await reacquisitionBlocked.promise
        expect(secondAcquired).toBeFalse()

        resumeRecovery.resolve()
        const firstLease = await first
        expect(secondAcquired).toBeFalse()
        await firstLease.release()
        const secondLease = await second
        await secondLease.release()
      } finally {
        __cargoTargetLockTestHooks.afterDeadLeaseOwnerRead = undefined
        __cargoTargetLockTestHooks.onRecoveryBlocked = undefined
        resumeRecovery.resolve()
      }
    })
  })

  test('serializes builds and grants only one prune lease', async () => {
    await withTarget(async (targetRoot) => {
      const blocked = deferred()
      let secondAcquired = false
      __cargoTargetLockTestHooks.onBuildBlocked = () => blocked.resolve()
      const firstBuild = await acquireCargoBuildLease(targetRoot)
      const secondBuildPromise = acquireCargoBuildLease(targetRoot).then((lease) => {
        secondAcquired = true
        return lease
      })
      try {
        await blocked.promise
        expect(secondAcquired).toBeFalse()
        await expect(acquireCargoPruneLease(targetRoot)).rejects.toThrow(
          'requires all shared-target builds to finish',
        )
      } finally {
        await firstBuild.release()
        const secondBuild = await secondBuildPromise
        await secondBuild.release()
        __cargoTargetLockTestHooks.onBuildBlocked = undefined
      }

      const results = await Promise.allSettled([
        acquireCargoPruneLease(targetRoot),
        acquireCargoPruneLease(targetRoot),
      ])
      expect(results.filter((result) => result.status === 'fulfilled')).toHaveLength(1)
      expect(results.filter((result) => result.status === 'rejected')).toHaveLength(1)
      for (const result of results) {
        if (result.status === 'fulfilled') await result.value.release()
      }
    })
  })
})
