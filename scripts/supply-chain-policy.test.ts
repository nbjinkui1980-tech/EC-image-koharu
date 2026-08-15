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

const releaseWorkflowFiles = ['build.yml', 'publish.yml', 'release.yml', 'lint.yml', 'test.yml'].map(
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

const releaseWorkflowUrl = new URL('../.github/workflows/release.yml', import.meta.url)

function releaseWorkflowText(): string {
  return readFileSync(releaseWorkflowUrl, 'utf8')
}

function topLevelPermissionLines(): string[] {
  const top = releaseWorkflowText().split(/^jobs:/m)[0] ?? ''
  const lines = top.split('\n')
  const start = lines.findIndex((line) => line === 'permissions:')
  if (start === -1) return []
  const out: string[] = []
  for (let i = start + 1; i < lines.length; i += 1) {
    const line = lines[i]!
    if (/^  [\w-]+:\s*[\w-]+\s*$/.test(line)) out.push(line.trim())
    else break
  }
  return out
}

function jobBlock(name: string): string {
  const lines = releaseWorkflowText().split('\n')
  const start = lines.findIndex((line) => line === `  ${name}:`)
  if (start === -1) return ''
  const out: string[] = []
  for (let i = start + 1; i < lines.length; i += 1) {
    const line = lines[i]!
    if (/^  \S/.test(line)) break
    out.push(line)
  }
  return out.join('\n')
}

test('release workflow keeps write permissions out of the top level', () => {
  expect(topLevelPermissionLines().filter((line) => line.endsWith(': write'))).toEqual([])
})

test('release workflow has no id-token grant anywhere', () => {
  expect(releaseWorkflowText()).not.toContain('id-token: write')
})

test('job permissions stay within the declared allowlist', () => {
  const release = jobBlock('release')
  const container = jobBlock('container')
  expect(release).toContain('contents: write')
  expect(release).toContain('actions: write')
  expect(release).not.toContain('packages: write')
  expect(container).toContain('packages: write')
  expect(container).toContain('actions: read')
  expect(container).not.toContain('contents: write')
})

test('trusted-signing-cli download verifies sha256 before execution', () => {
  const cli = jobBlock('release').match(/trusted-signing-cli[\s\S]*?(?=\n\s+- (?:name|uses):|$)/)
  expect(cli?.[0]).toContain('sha256')
  expect(cli?.[0]).toContain('39ece56f51f41eaf208cdf95323830cfa9e0a64c974ea9de8a27d82113d6e007')
})

test('artifact digest manifest uses the bare binary name', () => {
  const release = jobBlock('release')
  expect(release).toContain('sha256sum koharu >')
  expect(release).not.toContain('sha256sum target/release/koharu >')
})

const dockerfileUrl = new URL('../Dockerfile', import.meta.url)

test('dockerfile never downloads from releases/latest', () => {
  const dockerfile = readFileSync(dockerfileUrl, 'utf8')
  expect(dockerfile).not.toContain('releases/latest')
  expect(dockerfile).not.toContain('curl')
})

test('container job consumes only the same-run artifact with a digest check', () => {
  const container = jobBlock('container')
  expect(container).toContain('download-artifact')
  expect(container).toContain('sha256sum -c')
  const release = jobBlock('release')
  expect(release).toContain('upload-artifact')
  expect(release).toContain('koharu.sha256')
})

test('image name is pinned to the fork authority', () => {
  const text = releaseWorkflowText()
  expect(text).toContain('IMAGE_NAME: nbjinkui1980-tech/koharu')
  expect(text).not.toContain('IMAGE_NAME: ${{ github.repository }}')
})

test('container image carries OCI source/revision/version labels', () => {
  const container = jobBlock('container')
  expect(container).toContain(
    'org.opencontainers.image.source=https://github.com/nbjinkui1980-tech/koharu',
  )
  expect(container).toContain('org.opencontainers.image.revision=${{ github.sha }}')
  expect(container).toContain('org.opencontainers.image.version=${{ github.ref_name }}')
})

const ciWorkflowText = ['lint.yml', 'test.yml']
  .map((name) => readFileSync(new URL(`../.github/workflows/${name}`, import.meta.url), 'utf8'))
  .join('\n')

test('CI blocks on the full workspace rust gate set', () => {
  expect(ciWorkflowText).toContain('cargo fmt --all -- --check')
  expect(ciWorkflowText).toContain('cargo check --workspace --all-targets')
  expect(ciWorkflowText).toContain('cargo clippy --workspace --all-targets -- -D warnings')
  expect(ciWorkflowText).toContain('cargo test --workspace')
  expect(ciWorkflowText).toContain('--tests')
  // The rpc lib suite runs split around routes::pages:: until the Linux
  // heap-corruption-on-exit is root-caused (see test.yml KNOWN ISSUE note).
  expect(ciWorkflowText).toContain('-p koharu-rpc --lib')
})

test('CI blocks on the UI format/lint/test gate set', () => {
  expect(ciWorkflowText).toContain('format:check')
  expect(ciWorkflowText).toMatch(/bun (run )?lint:ui/)
  expect(ciWorkflowText).toMatch(/bun run --filter ui test|test:ui/)
})

test('CI blocks on generated-artifact drift', () => {
  expect(ciWorkflowText).toContain('check:generated')
})

test('CI blocks on cargo and dependency audit', () => {
  expect(ciWorkflowText).toContain('cargo audit')
  expect(ciWorkflowText).toContain('audit:deps')
})
