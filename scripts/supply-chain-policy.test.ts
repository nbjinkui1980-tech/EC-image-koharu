import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

import { evaluateAudit, validateAllowlist } from './supply-chain-policy'

const futureDate = '2026-12-31'
const audit = {
  'example-package': [
    {
      url: 'https://github.com/advisories/GHSA-aaaa-bbbb-cccc',
      severity: 'high',
    },
  ],
}

const developmentException = {
  package: 'example-package',
  advisory: 'GHSA-aaaa-bbbb-cccc',
  severity: 'high',
  dependencyPath: 'ui > example-package',
  reachability: 'development',
  owner: 'security@example.invalid',
  reason: 'fixture only',
  expiresOn: futureDate,
}

test('rejects unknown audit findings', () => {
  expect(evaluateAudit(audit, [], new Date('2026-08-10T00:00:00Z')).ok).toBe(false)
})

test('requires complete, unexpired allowlist metadata', () => {
  expect(() =>
    validateAllowlist([{ ...developmentException, owner: '' }], new Date('2026-08-10T00:00:00Z')),
  ).toThrow('owner')
  expect(() =>
    validateAllowlist(
      [{ ...developmentException, expiresOn: '2026-08-10' }],
      new Date('2026-08-10T00:00:00Z'),
    ),
  ).toThrow('expired')
})

test('never permits a runtime reachable high or critical finding', () => {
  const allowlist = [{ ...developmentException, reachability: 'runtime' }]

  expect(evaluateAudit(audit, allowlist, new Date('2026-08-10T00:00:00Z')).ok).toBe(false)
})

test('permits only a complete future-dated development exception', () => {
  expect(evaluateAudit(audit, [developmentException], new Date('2026-08-10T00:00:00Z')).ok).toBe(
    true,
  )
})

const releaseWorkflowFiles = ['build.yml', 'publish.yml', 'release.yml'].map(
  (name) => new URL(`../.github/workflows/${name}`, import.meta.url),
)

function workflowUses(file: URL): string[] {
  return readFileSync(file, 'utf8')
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.startsWith('- uses:') || line.startsWith('uses:'))
    .map((line) => line.replace(/^-?\s*uses:\s*/, '').trim())
}

test('release workflows pin every non-local action to a full commit sha', () => {
  const offenders: string[] = []
  for (const file of releaseWorkflowFiles) {
    for (const use of workflowUses(file)) {
      if (use.startsWith('./')) continue
      const ref = use.split('@')[1]?.split(/\s/)[0] ?? ''
      if (!/^[0-9a-f]{40}$/.test(ref)) offenders.push(use)
    }
  }
  expect(offenders).toEqual([])
})

test('pinned actions carry the version comment', () => {
  const offenders: string[] = []
  for (const file of releaseWorkflowFiles) {
    for (const use of workflowUses(file)) {
      if (use.startsWith('./')) continue
      const ref = use.split('@')[1] ?? ''
      const [, ...rest] = ref.split(/\s+/)
      if (!/^#\s*\S+/.test(rest.join(' '))) offenders.push(use)
    }
  }
  expect(offenders).toEqual([])
})
