// @ts-nocheck
import { exec as execCallback, spawn, type ChildProcess } from 'node:child_process'
import { readdir, access, mkdir } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'

import { acquireCargoBuildLease } from './cargo-target-lock'
import { resolveVerifiedSharedTarget } from './storage'

const exec = promisify(execCallback)
const terminationSignals = ['SIGINT', 'SIGTERM', 'SIGHUP'] as const

function processGroupExists(pid: number) {
  try {
    process.kill(-pid, 0)
    return true
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code
    if (code === 'EPERM' || code === 'ESRCH') return false
    throw error
  }
}

export async function terminateOwnedProcessTree(
  child: ChildProcess,
  signal: NodeJS.Signals = 'SIGTERM',
) {
  if (!child.pid) return

  if (process.platform === 'win32') {
    await new Promise<void>((resolve) => {
      const taskkill = spawn('taskkill', ['/pid', String(child.pid), '/T', '/F'], {
        stdio: 'ignore',
        windowsHide: true,
      })
      taskkill.once('error', () => {
        child.kill(signal)
        resolve()
      })
      taskkill.once('exit', () => resolve())
    })
    return
  }

  try {
    process.kill(-child.pid, signal)
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code
    if (code === 'ESRCH') return
    if (code === 'EPERM') {
      child.kill(signal)
      return
    }
    throw error
  }

  for (let attempt = 0; attempt < 20 && processGroupExists(child.pid); attempt += 1) {
    await new Promise((resolve) => setTimeout(resolve, 25))
  }
  if (processGroupExists(child.pid)) process.kill(-child.pid, 'SIGKILL')
}

async function pathExists(target: string) {
  try {
    await access(target)
    return true
  } catch {
    return false
  }
}

async function checkNvcc() {
  try {
    await exec('nvcc --version', { env: process.env })
  } catch {
    throw new Error('nvcc not found')
  }
}

function sortVersionsDesc(versions: string[]) {
  return versions.sort((a, b) => b.localeCompare(a, undefined, { numeric: true }))
}

async function setupCuda() {
  const cudaPath = process.env.CUDA_PATH
  if (cudaPath) {
    const binPath = path.join(cudaPath, 'bin')
    process.env.PATH = `${binPath}${path.delimiter}${process.env.PATH}`
    return
  }

  const cudaRoot = 'C:/Program Files/NVIDIA GPU Computing Toolkit/CUDA'
  const versions = await readdir(cudaRoot).catch(() => [])

  sortVersionsDesc(versions)

  for (const version of versions) {
    if (version.startsWith('v')) {
      const binPath = path.join(cudaRoot, version, 'bin')
      if (await pathExists(binPath)) {
        process.env.PATH = `${binPath}${path.delimiter}${process.env.PATH}`
        process.env.CUDA_PATH = path.join(cudaRoot, version)
        return
      }
    }
  }

  throw new Error(
    'NVCC not found. Please install the CUDA Toolkit from https://developer.nvidia.com/cuda-downloads',
  )
}

async function setupCl() {
  const vsRoots = [
    'C:/Program Files/Microsoft Visual Studio',
    'C:/Program Files (x86)/Microsoft Visual Studio',
  ]
  const editions = ['Community', 'Professional', 'Enterprise', 'BuildTools']

  for (const vsRoot of vsRoots) {
    const vsVersions = await readdir(vsRoot).catch(() => [])

    for (const vsVersion of vsVersions) {
      for (const edition of editions) {
        const vcPath = path.join(vsRoot, vsVersion, edition, 'VC/Tools/MSVC')
        if (await pathExists(vcPath)) {
          const msvcVersions = await readdir(vcPath)
          for (const msvcVersion of msvcVersions) {
            const binPath = path.join(vcPath, msvcVersion, 'bin/Hostx64/x64')
            if (await pathExists(binPath)) {
              process.env.PATH = `${binPath}${path.delimiter}${process.env.PATH}`
              return
            }
          }
        }
      }
    }
  }

  throw new Error(
    'cl.exe not found. Please install Visual Studio with C++ build tools from https://visualstudio.microsoft.com/downloads/',
  )
}

async function dev() {
  const args = process.argv.slice(2)
  if (args.length === 0) throw new Error('No command provided')

  const runsRustBuild = args[0] === 'cargo' || args[0] === 'tauri'
  if (runsRustBuild) {
    process.env.CARGO_TARGET_DIR = await resolveVerifiedSharedTarget(
      process.env.KOHARU_SHARED_TARGET_DIR,
    )
    process.env.KOHARU_CARGO_GUARD_ACTIVE = '1'
  }

  if (process.env.KOHARU_TMPDIR) {
    await mkdir(process.env.KOHARU_TMPDIR, { recursive: true })
    process.env.TMPDIR = process.env.KOHARU_TMPDIR
    process.env.TMP = process.env.KOHARU_TMPDIR
    process.env.TEMP = process.env.KOHARU_TMPDIR
  }

  if (os.type() === 'Windows_NT') {
    // First, try to check if nvcc is available
    await checkNvcc()
      // If not found, try to set up CUDA paths
      .catch(async () => {
        await setupCuda()
        // Check again after setup
        await checkNvcc()
      })

    // Setup cl.exe path
    await setupCl()
  }

  const targetLease =
    process.env.CARGO_TARGET_DIR && runsRustBuild
      ? await acquireCargoBuildLease(process.env.CARGO_TARGET_DIR)
      : undefined
  let proc: ChildProcess | undefined
  let cleanup: Promise<void> | undefined
  const terminate = (signal: NodeJS.Signals = 'SIGTERM') => {
    if (!proc) return Promise.resolve()
    return (cleanup ??= terminateOwnedProcessTree(proc, signal))
  }
  const terminateOnSignal = (signal: NodeJS.Signals) => {
    void terminate(signal).catch((error) => {
      try {
        proc?.kill(signal)
      } catch {}
      process.stderr.write(`Error: failed to terminate child process tree: ${error.message}\n`)
    })
  }

  try {
    const code = await new Promise<number>((resolve, reject) => {
      proc = spawn(args[0], args.slice(1), {
        stdio: 'inherit',
        shell: false,
        env: process.env,
        detached: process.platform !== 'win32',
      })

      for (const signal of terminationSignals) process.once(signal, terminateOnSignal)

      proc.once('error', reject)
      proc.once('exit', (exitCode) => resolve(exitCode ?? 1))
    })
    process.exitCode = code
  } finally {
    for (const signal of terminationSignals) process.removeListener(signal, terminateOnSignal)
    try {
      await terminate()
    } finally {
      await targetLease?.release()
    }
  }
}

if (import.meta.main) {
  dev().catch((err) => {
    process.stderr.write(`Error: ${err.message} \n`)
    process.exit(1)
  })
}
