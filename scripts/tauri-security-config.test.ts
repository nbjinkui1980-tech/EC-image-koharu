import { describe, expect, test } from 'bun:test'
import { readFile } from 'node:fs/promises'

const conf = JSON.parse(
  await readFile(new URL('../crates/koharu/tauri.conf.json', import.meta.url), 'utf8'),
) as { app?: { security?: { csp?: unknown } } }

const FROZEN_DIRECTIVES = [
  "default-src 'self'",
  "object-src 'none'",
  "base-uri 'none'",
  "frame-ancestors 'none'",
  "form-action 'none'",
]

// AR07-T01 policy: the Tauri security config carries the frozen CSP.
describe('tauri security config policy', () => {
  test('csp is configured and carries the frozen directives', () => {
    const csp = conf.app?.security?.csp
    expect(typeof csp).toBe('string')
    for (const directive of FROZEN_DIRECTIVES) {
      expect(csp as string).toContain(directive)
    }
  })
})
