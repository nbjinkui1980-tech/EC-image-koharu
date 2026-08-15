import { test } from 'bun:test'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { createConnection, createServer } from 'node:net'

import { terminateOwnedProcessTree } from './dev'

const grandchildScript = `
  const { createServer } = require('node:net')
  const server = createServer()
  server.listen(Number(process.argv[1]), '127.0.0.1')
`

const parentScript = `
  const { spawn } = require('node:child_process')
  spawn(process.execPath, ['-e', ${JSON.stringify(grandchildScript)}, process.argv[1]], {
    stdio: 'ignore',
  })
  setInterval(() => {}, 1000)
`

async function freePort() {
  const server = createServer()
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  const address = server.address()
  if (!address || typeof address === 'string') throw new Error('expected TCP address')
  await new Promise<void>((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  )
  return address.port
}

async function waitForPort(port: number) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const connected = await new Promise<boolean>((resolve) => {
      const socket = createConnection(port, '127.0.0.1')
      socket.once('connect', () => {
        socket.destroy()
        resolve(true)
      })
      socket.once('error', () => resolve(false))
    })
    if (connected) return
    await Bun.sleep(20)
  }
  throw new Error(`dev-server grandchild did not bind port ${port}`)
}

function bind(port: number) {
  return new Promise<void>((resolve, reject) => {
    const server = createServer()
    server.once('error', reject)
    server.listen(port, '127.0.0.1', () =>
      server.close((error) => (error ? reject(error) : resolve())),
    )
  })
}

async function waitForBind(port: number) {
  let lastError: unknown
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      await bind(port)
      return
    } catch (error) {
      lastError = error
      await Bun.sleep(20)
    }
  }
  throw lastError
}

test('releases a dev-server grandchild port when the owned process tree exits', async () => {
  const port = await freePort()
  const parent = spawn(process.execPath, ['-e', parentScript, String(port)], {
    detached: process.platform !== 'win32',
    stdio: 'ignore',
  })
  parent.unref()
  let cleaned = false

  try {
    await waitForPort(port)

    await terminateOwnedProcessTree(parent)
    cleaned = true
    await waitForBind(port)
  } finally {
    if (!cleaned) await terminateOwnedProcessTree(parent, 'SIGKILL')
  }
})

test.skipIf(process.platform === 'win32')(
  'falls back when process-group termination is denied',
  async () => {
    const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], {
      detached: true,
      stdio: 'ignore',
    })
    const exited = once(child, 'exit')
    const kill = process.kill
    process.kill = ((pid: number, signal?: NodeJS.Signals | number) => {
      if (pid === -child.pid!) throw Object.assign(new Error('denied'), { code: 'EPERM' })
      return kill(pid, signal)
    }) as typeof process.kill

    try {
      await terminateOwnedProcessTree(child)
      await exited
    } finally {
      process.kill = kill
      child.kill('SIGKILL')
    }
  },
)
