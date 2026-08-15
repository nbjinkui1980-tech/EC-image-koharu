import { describe, expect, test } from 'bun:test'
import { readFile } from 'node:fs/promises'

const conf = JSON.parse(
  await readFile(new URL('../crates/koharu/tauri.conf.json', import.meta.url), 'utf8'),
) as { app?: { security?: { csp?: unknown } } }

const capabilities = JSON.parse(
  await readFile(new URL('../crates/koharu/capabilities/default.json', import.meta.url), 'utf8'),
) as { permissions?: Array<string | { identifier?: string; allow?: unknown }> }

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

  // AR07-T03 RED: the wildcard fs scope must be gone; dialog-picked files get
  // a runtime temporary scope from the OS gesture, so no persisted wildcard
  // is needed.
  test('no wildcard fs scope remains; dialog and fs commands stay', () => {
    const permissions = capabilities.permissions ?? []
    const scopeEntries = permissions.filter(
      (entry): entry is { identifier?: string; allow?: Array<{ path?: string }> } =>
        typeof entry === 'object' && entry.identifier === 'fs:scope',
    )
    const allowsWildcard = scopeEntries.some((entry) =>
      (entry.allow ?? []).some((item) => item.path === '**'),
    )
    expect(allowsWildcard).toBe(false)

    const names = permissions.map((entry) =>
      typeof entry === 'string' ? entry : (entry.identifier ?? ''),
    )
    for (const required of [
      'dialog:allow-open',
      'dialog:allow-save',
      'fs:allow-read-file',
      'fs:allow-write-file',
      'fs:allow-mkdir',
      'fs:allow-exists',
    ]) {
      expect(names).toContain(required)
    }
  })
})
