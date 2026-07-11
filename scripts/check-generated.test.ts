import { afterEach, expect, test } from 'bun:test'
import { rm, writeFile } from 'node:fs/promises'
import path from 'node:path'

const root = path.resolve(import.meta.dir, '..')
const probePath = 'ui/lib/api/schemas/__generated_drift_probe__.ts'
const probe = path.join(root, probePath)
const mockProbePath = 'ui/lib/api/generated.msw.ts'
const mockProbe = path.join(root, mockProbePath)
const stagedProbePath = 'ui/lib/api/schemas/__staged_generated_drift_probe__.ts'
const stagedProbe = path.join(root, stagedProbePath)

afterEach(async () => {
  Bun.spawnSync({
    cmd: ['git', 'rm', '--cached', '--force', '--ignore-unmatch', '--', stagedProbePath],
    cwd: root,
    stdout: 'ignore',
    stderr: 'ignore',
  })
  await Promise.all([
    rm(probe, { force: true }),
    rm(mockProbe, { force: true }),
    rm(stagedProbe, { force: true }),
  ])
})

async function runChecker() {
  const child = Bun.spawn([process.execPath, 'scripts/check-generated.ts'], {
    cwd: root,
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const [exitCode, stdout, stderr] = await Promise.all([
    child.exited,
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ])

  return { exitCode, output: `${stdout}\n${stderr}` }
}

test('rejects untracked generated schemas', async () => {
  await writeFile(probe, 'export {}\n')

  const result = await runChecker()

  expect(result.exitCode).not.toBe(0)
  expect(result.output).toContain(probePath)
})

test('rejects an untracked generated mock client', async () => {
  await writeFile(mockProbe, 'export {}\n')

  const result = await runChecker()

  expect(result.exitCode).not.toBe(0)
  expect(result.output).toContain(mockProbePath)
})

test('rejects a staged generated artifact', async () => {
  await writeFile(stagedProbe, 'export {}\n')
  const staged = Bun.spawnSync({
    cmd: ['git', 'add', '--', stagedProbePath],
    cwd: root,
    stdout: 'ignore',
    stderr: 'pipe',
  })
  expect(staged.exitCode).toBe(0)

  const result = await runChecker()

  expect(result.exitCode).not.toBe(0)
  expect(result.output).toContain(stagedProbePath)
})
