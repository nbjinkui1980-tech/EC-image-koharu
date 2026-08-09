import { expect, test } from 'bun:test'

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
