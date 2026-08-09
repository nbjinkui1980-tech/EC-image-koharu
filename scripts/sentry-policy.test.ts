import { expect, test } from 'bun:test'
import { readFile } from 'node:fs/promises'
import path from 'node:path'

const root = path.resolve(import.meta.dir, '..')

async function source(file: string) {
  return readFile(path.join(root, file), 'utf8')
}

test('desktop and browser Sentry disable default PII', async () => {
  const [rust, browser] = await Promise.all([
    source('crates/koharu/src/sentry.rs'),
    source('ui/instrumentation-client.ts'),
  ])

  expect(rust).toContain('send_default_pii: false')
  expect(browser).toContain('sendDefaultPii: false')
  expect(rust).not.toContain('send_default_pii: true')
  expect(browser).not.toContain('sendDefaultPii: true')
})

test('browser error boundaries neither render nor capture the original error', async () => {
  const [boundary, globalError] = await Promise.all([
    source('ui/components/AppErrorBoundary.tsx'),
    source('ui/app/global-error.tsx'),
  ])

  expect(boundary).not.toContain('error.message')
  expect(boundary).not.toMatch(/captureException\(error\)/)
  expect(boundary).toContain("Sentry.captureException(new Error('Application error'))")
  expect(globalError).not.toContain('error.message')
  expect(globalError).not.toMatch(/captureException\(error\)/)
  expect(globalError).toContain("Sentry.captureException(new Error('Application error'))")
})
