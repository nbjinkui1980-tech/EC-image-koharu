import { describe, expect, test } from 'bun:test'

import { GIB, storageViolations } from './storage'

describe('storageViolations', () => {
  test('enforces the free-space and generated-cache ceilings', () => {
    expect(
      storageViolations({
        freeBytes: 19 * GIB,
        targetBytes: 17 * GIB,
        nextBytes: 2 * GIB,
      }),
    ).toEqual([
      'Available disk space is below 20 GiB.',
      'target exceeds 16 GiB.',
      'ui/.next exceeds 1 GiB.',
    ])
  })

  test('accepts a workspace below every ceiling', () => {
    expect(
      storageViolations({
        freeBytes: 20 * GIB,
        targetBytes: 16 * GIB,
        nextBytes: 1 * GIB,
      }),
    ).toEqual([])
  })
})
