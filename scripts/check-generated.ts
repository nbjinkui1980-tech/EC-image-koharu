import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('..', import.meta.url))
const generatedPaths = [
  'ui/openapi.json',
  'ui/lib/api/generated.ts',
  'ui/lib/api/generated.msw.ts',
  'ui/lib/api/schemas',
]

const status = Bun.spawnSync({
  cmd: ['git', 'status', '--porcelain=v1', '--untracked-files=all', '--', ...generatedPaths],
  cwd: root,
  stdout: 'pipe',
  stderr: 'inherit',
})
const changes = status.stdout.toString().trim()

if (changes) {
  process.stderr.write(`Generated artifact drift:\n${changes}\n`)
}

process.exit(status.exitCode === 0 && !changes ? 0 : 1)
