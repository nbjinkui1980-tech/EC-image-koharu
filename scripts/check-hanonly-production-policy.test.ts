import { afterEach, describe, expect, test } from 'bun:test'
import { createHash } from 'node:crypto'
import { open, realpath } from 'node:fs/promises'
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rename,
  rm,
  stat,
  symlink,
  unlink,
  writeFile,
} from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

import {
  formatCliFailure,
  metadataVersion,
  PolicyError,
  readBaselineSnapshot,
  readRepoText,
  readStableFile,
  repoRoot,
  r51EvidenceExecutable,
  r51MarkerInventoryCommand,
  validateB0Authorization,
  validateB0SourceGateAntiFixture,
  validateDependencyInventory,
  validateFrozenInterpreterRecords,
  validateGeneratedRustAudit,
  validateRedTestState,
  validateReleaseFeatureInventory,
  validateSecureRegularStat,
  type DependencyInventoryInput,
  type FrozenInterpreterRecord,
  type GeneratedRustFile,
  type JsonObject,
  type RustSourceFile,
  type SnapshotMetadata,
} from './check-hanonly-production-policy'

const registrySource = 'registry+https://github.com/rust-lang/crates.io-index'
const temporaryRoots: string[] = []

afterEach(async () => {
  await Promise.all(
    temporaryRoots.splice(0).map((root) => rm(root, { force: true, recursive: true })),
  )
})

function dependency(
  name: string,
  kind: 'normal' | 'dev' | 'build',
  req: string,
  features: string[],
): JsonObject {
  return {
    name,
    source: registrySource,
    req,
    kind,
    rename: null,
    optional: false,
    uses_default_features: true,
    features,
    target: null,
    registry: null,
  }
}

function packageRecord(
  name: string,
  version: string,
  dependencies: string[],
  checksum?: string,
): JsonObject {
  return {
    name,
    version,
    source: registrySource,
    checksum: checksum ?? `${name}-checksum`,
    dependencies,
  }
}

function validInventory(): DependencyInventoryInput {
  const rustix = packageRecord('rustix', '1.1.4', ['libc'], 'rustix-baseline-checksum')
  const sha2 = packageRecord('sha2', '0.10.9', ['digest'], 'sha2-baseline-checksum')
  const other = packageRecord('other', '1.0.0', ['base'])
  const baselineApp = { name: 'koharu-app', version: '0.61.2', dependencies: ['base'] }
  const baselineLlm = { name: 'koharu-llm', version: '0.61.2', dependencies: ['base'] }
  return {
    cargoMetadata: {
      packages: [
        {
          name: 'koharu-app',
          dependencies: [
            dependency('rustix', 'dev', '=1.1.4', ['fs']),
            dependency('sha2', 'dev', '=0.10.9', []),
          ],
        },
        {
          name: 'koharu-llm',
          dependencies: [dependency('sha2', 'build', '=0.10.9', [])],
        },
        { name: 'other', dependencies: [] },
      ],
    },
    rootManifest: { workspace: { dependencies: {} } },
    appManifest: {
      dependencies: { anyhow: '1' },
      'dev-dependencies': {
        rustix: { version: '=1.1.4', features: ['fs'] },
        sha2: '=0.10.9',
      },
    },
    llmManifest: {
      dependencies: { anyhow: '1' },
      'build-dependencies': { sha2: '=0.10.9' },
    },
    baselineLock: {
      version: 4,
      package: [baselineApp, baselineLlm, rustix, sha2, other],
    },
    currentLock: {
      version: 4,
      package: [
        {
          ...baselineApp,
          dependencies: ['base', 'rustix 1.1.4', 'sha2'],
        },
        { ...baselineLlm, dependencies: ['base', 'sha2'] },
        structuredClone(rustix),
        structuredClone(sha2),
        structuredClone(other),
      ],
    },
  }
}

function metadataPackages(input: DependencyInventoryInput): JsonObject[] {
  return (input.cargoMetadata as JsonObject).packages as JsonObject[]
}

function lockPackages(input: DependencyInventoryInput, which = 'currentLock'): JsonObject[] {
  return (input[which as 'currentLock'] as JsonObject).package as JsonObject[]
}

function packageNamed(packages: JsonObject[], name: string): JsonObject {
  return packages.find((record) => record.name === name)!
}

type InventoryCase = {
  name: string
  mutate: (input: DependencyInventoryInput) => void
  category: string
}

const inventoryCases: InventoryCase[] = [
  {
    name: 'rejects a missing koharu-app rustix edge',
    mutate(input) {
      ;(metadataPackages(input)[0].dependencies as JsonObject[]).shift()
    },
    category: 'metadata-policy',
  },
  {
    name: 'rejects a missing koharu-app sha2 edge',
    mutate(input) {
      ;(metadataPackages(input)[0].dependencies as JsonObject[]).pop()
    },
    category: 'metadata-policy',
  },
  {
    name: 'rejects a missing koharu-llm sha2 edge',
    mutate(input) {
      metadataPackages(input)[1].dependencies = []
    },
    category: 'metadata-policy',
  },
  {
    name: 'rejects semver drift',
    mutate(input) {
      ;((metadataPackages(input)[0].dependencies as JsonObject[])[0] as JsonObject).req = '^1.1.4'
    },
    category: 'metadata-policy',
  },
  {
    name: 'rejects a missing rustix feature',
    mutate(input) {
      ;((metadataPackages(input)[0].dependencies as JsonObject[])[0] as JsonObject).features = []
    },
    category: 'metadata-policy',
  },
  {
    name: 'rejects an extra rustix feature',
    mutate(input) {
      ;((metadataPackages(input)[0].dependencies as JsonObject[])[0] as JsonObject).features = [
        'fs',
        'process',
      ]
    },
    category: 'metadata-policy',
  },
  {
    name: 'rejects promotion to a normal dependency',
    mutate(input) {
      ;((metadataPackages(input)[0].dependencies as JsonObject[])[1] as JsonObject).kind = 'normal'
    },
    category: 'metadata-policy',
  },
  {
    name: 'rejects an alias on an expected metadata edge',
    mutate(input) {
      ;((metadataPackages(input)[0].dependencies as JsonObject[])[1] as JsonObject).rename =
        'audit-hash'
    },
    category: 'metadata-policy',
  },
  {
    name: 'rejects a root workspace declaration',
    mutate(input) {
      ;(
        ((input.rootManifest as JsonObject).workspace as JsonObject).dependencies as JsonObject
      ).sha2 = '=0.10.9'
    },
    category: 'manifest-policy',
  },
  {
    name: 'rejects a root workspace sha2 alias',
    mutate(input) {
      ;(((input.rootManifest as JsonObject).workspace as JsonObject).dependencies as JsonObject)[
        'audit-hash'
      ] = { package: 'sha2', version: '=0.10.9' }
    },
    category: 'manifest-policy',
  },
  {
    name: 'rejects a root workspace rustix alias',
    mutate(input) {
      ;(((input.rootManifest as JsonObject).workspace as JsonObject).dependencies as JsonObject)[
        'audit-fs'
      ] = { package: 'rustix', version: '=1.1.4' }
    },
    category: 'manifest-policy',
  },
  {
    name: 'rejects a policy dependency key even when package points elsewhere',
    mutate(input) {
      ;(
        ((input.rootManifest as JsonObject).workspace as JsonObject).dependencies as JsonObject
      ).sha2 = { package: 'other', version: '=1.0.0' }
    },
    category: 'manifest-policy',
  },
  {
    name: 'rejects a target-specific sha2 alias in koharu-app',
    mutate(input) {
      ;(input.appManifest as JsonObject).target = {
        'cfg(unix)': {
          dependencies: { 'audit-hash': { package: 'sha2', version: '=0.10.9' } },
        },
      }
    },
    category: 'manifest-policy',
  },
  {
    name: 'rejects an aliased expected koharu-app rustix edge',
    mutate(input) {
      const dev = (input.appManifest as JsonObject)['dev-dependencies'] as JsonObject
      dev['audit-fs'] = { package: 'rustix', version: '=1.1.4', features: ['fs'] }
      delete dev.rustix
    },
    category: 'manifest-policy',
  },
  {
    name: 'rejects koharu-llm sha2 in dependencies',
    mutate(input) {
      const manifest = input.llmManifest as JsonObject
      ;(manifest.dependencies as JsonObject).sha2 = '=0.10.9'
      delete (manifest['build-dependencies'] as JsonObject).sha2
    },
    category: 'manifest-policy',
  },
  {
    name: 'rejects a fourth related edge in another package',
    mutate(input) {
      ;(metadataPackages(input)[2].dependencies as JsonObject[]).push(
        dependency('sha2', 'normal', '=0.10.9', []),
      )
    },
    category: 'metadata-policy',
  },
  {
    name: 'rejects an aliased fourth metadata edge',
    mutate(input) {
      const edge = dependency('sha2', 'normal', '=0.10.9', [])
      edge.rename = 'audit-hash'
      ;(metadataPackages(input)[2].dependencies as JsonObject[]).push(edge)
    },
    category: 'metadata-policy',
  },
  {
    name: 'rejects an unrelated package dependency-list change',
    mutate(input) {
      ;(packageNamed(lockPackages(input), 'other').dependencies as string[]).push('drift')
    },
    category: 'lock-policy',
  },
  {
    name: 'rejects an added package',
    mutate(input) {
      lockPackages(input).push(packageRecord('added', '1.0.0', []))
    },
    category: 'lock-policy',
  },
  {
    name: 'rejects a duplicate lock package key',
    mutate(input) {
      for (const which of ['baselineLock', 'currentLock'] as const) {
        const packages = (input[which] as JsonObject).package as JsonObject[]
        packages.push(structuredClone(packageNamed(packages, 'other')))
      }
    },
    category: 'lock-policy',
  },
  {
    name: 'rejects package version drift',
    mutate(input) {
      packageNamed(lockPackages(input), 'other').version = '2.0.0'
    },
    category: 'lock-policy',
  },
  {
    name: 'rejects package source drift',
    mutate(input) {
      packageNamed(lockPackages(input), 'other').source = 'git+https://example.invalid/repo'
    },
    category: 'lock-policy',
  },
  {
    name: 'rejects package checksum drift',
    mutate(input) {
      packageNamed(lockPackages(input), 'other').checksum = 'drift'
    },
    category: 'lock-policy',
  },
  {
    name: 'rejects an extra koharu-app lock dependency',
    mutate(input) {
      ;(packageNamed(lockPackages(input), 'koharu-app').dependencies as string[]).push('extra')
    },
    category: 'lock-policy',
  },
  {
    name: 'rejects a duplicate koharu-app allowed lock dependency',
    mutate(input) {
      ;(packageNamed(lockPackages(input), 'koharu-app').dependencies as string[]).push('sha2')
    },
    category: 'lock-policy',
  },
  {
    name: 'rejects an extra koharu-llm lock dependency',
    mutate(input) {
      ;(packageNamed(lockPackages(input), 'koharu-llm').dependencies as string[]).push('extra')
    },
    category: 'lock-policy',
  },
  {
    name: 'rejects a duplicate koharu-llm allowed lock dependency',
    mutate(input) {
      ;(packageNamed(lockPackages(input), 'koharu-llm').dependencies as string[]).push('sha2')
    },
    category: 'lock-policy',
  },
  {
    name: 'rejects a changed existing rustix 1.1.4 record',
    mutate(input) {
      packageNamed(lockPackages(input), 'rustix').checksum = 'wrong-rustix'
    },
    category: 'lock-policy',
  },
  {
    name: 'rejects a changed existing sha2 0.10.9 record',
    mutate(input) {
      packageNamed(lockPackages(input), 'sha2').checksum = 'wrong-sha2'
    },
    category: 'lock-policy',
  },
]

describe('dependency inventory policy', () => {
  test('accepts the canonical dependency inventory', () => {
    expect(() => validateDependencyInventory(validInventory())).not.toThrow()
  })

  test.each(inventoryCases)('$name', ({ mutate, category }) => {
    const input = validInventory()
    mutate(input)

    expect(() => validateDependencyInventory(input)).toThrow(
      expect.objectContaining({ category }) as PolicyError,
    )
  })
})

async function makeSnapshot(
  mutateMetadata?: (metadata: JsonObject) => void,
): Promise<{ root: string; snapshot: string; lockPath: string }> {
  const temporaryRoot = await realpath(await mkdtemp(path.join(os.tmpdir(), 'hanonly-policy-')))
  temporaryRoots.push(temporaryRoot)
  const root = path.join(temporaryRoot, 'repo')
  const snapshot = path.join(temporaryRoot, 'snapshot')
  await Promise.all([mkdir(root), mkdir(snapshot)])
  const lockPath = path.join(snapshot, 'pre-edit-Cargo.lock')
  const bytes = Buffer.from('version = 4\npackage = []\n')
  await writeFile(lockPath, bytes, { mode: 0o600 })
  await chmod(lockPath, 0o600)
  const lockStat = await stat(lockPath)
  const metadata: JsonObject = {
    mode: '0600',
    owner_uid: process.getuid!(),
    path: 'pre-edit-Cargo.lock',
    sha256: createHash('sha256').update(bytes).digest('hex'),
    st_dev: lockStat.dev,
    st_ino: lockStat.ino,
    type: 'regular',
    version: metadataVersion,
  } satisfies SnapshotMetadata
  mutateMetadata?.(metadata)
  const metadataPath = path.join(snapshot, 'pre-edit-Cargo.lock.metadata.json')
  await writeFile(metadataPath, JSON.stringify(metadata), { mode: 0o600 })
  await chmod(metadataPath, 0o600)
  return { root, snapshot, lockPath }
}

type SnapshotCase = {
  name: string
  mutate: (metadata: JsonObject) => void
  category: string
}

const snapshotCases: SnapshotCase[] = [
  {
    name: 'rejects an unknown metadata key',
    mutate(metadata) {
      metadata.unknown = true
    },
    category: 'snapshot-metadata',
  },
  {
    name: 'rejects a missing metadata key',
    mutate(metadata) {
      delete metadata.sha256
    },
    category: 'snapshot-metadata',
  },
  {
    name: 'rejects metadata hash drift',
    mutate(metadata) {
      metadata.sha256 = '0'.repeat(64)
    },
    category: 'snapshot-hash',
  },
  {
    name: 'rejects metadata owner drift',
    mutate(metadata) {
      metadata.owner_uid = process.getuid!() + 1
    },
    category: 'snapshot-metadata',
  },
  {
    name: 'rejects metadata mode drift',
    mutate(metadata) {
      metadata.mode = '0644'
    },
    category: 'snapshot-metadata',
  },
  {
    name: 'rejects metadata type drift',
    mutate(metadata) {
      metadata.type = 'directory'
    },
    category: 'snapshot-metadata',
  },
  {
    name: 'rejects metadata device drift',
    mutate(metadata) {
      metadata.st_dev = Number(metadata.st_dev) + 1
    },
    category: 'snapshot-identity',
  },
  {
    name: 'rejects metadata inode drift',
    mutate(metadata) {
      metadata.st_ino = Number(metadata.st_ino) + 1
    },
    category: 'snapshot-identity',
  },
]

describe('snapshot trust boundary', () => {
  test('reads a valid secure baseline snapshot', async () => {
    const fixture = await makeSnapshot()

    await expect(readBaselineSnapshot(fixture.snapshot, fixture.root)).resolves.toEqual({
      version: 4,
      package: [],
    })
  })

  test.each(snapshotCases)('$name', async ({ mutate, category }) => {
    const fixture = await makeSnapshot(mutate)

    await expect(readBaselineSnapshot(fixture.snapshot, fixture.root)).rejects.toMatchObject({
      category,
    })
  })

  test('rejects a missing baseline lock', async () => {
    const fixture = await makeSnapshot()
    await unlink(fixture.lockPath)

    await expect(readBaselineSnapshot(fixture.snapshot, fixture.root)).rejects.toMatchObject({
      category: 'baseline-missing',
    })
  })

  test('rejects a final-component lock symlink', async () => {
    const fixture = await makeSnapshot()
    const target = path.join(path.dirname(fixture.snapshot), 'target.lock')
    await writeFile(target, 'version = 4\npackage = []\n', { mode: 0o600 })
    await chmod(target, 0o600)
    await unlink(fixture.lockPath)
    await symlink(target, fixture.lockPath)

    await expect(readBaselineSnapshot(fixture.snapshot, fixture.root)).rejects.toMatchObject({
      category: 'baseline-missing',
    })
  })

  test('rejects a final-component metadata symlink', async () => {
    const fixture = await makeSnapshot()
    const metadataPath = path.join(fixture.snapshot, 'pre-edit-Cargo.lock.metadata.json')
    const target = path.join(path.dirname(fixture.snapshot), 'metadata.json')
    await rename(metadataPath, target)
    await symlink(target, metadataPath)

    await expect(readBaselineSnapshot(fixture.snapshot, fixture.root)).rejects.toMatchObject({
      category: 'snapshot-metadata',
    })
  })

  test('rejects a symlinked snapshot directory', async () => {
    const fixture = await makeSnapshot()
    const alias = path.join(path.dirname(fixture.snapshot), 'snapshot-alias')
    await symlink(fixture.snapshot, alias)

    await expect(readBaselineSnapshot(alias, fixture.root)).rejects.toMatchObject({
      category: 'snapshot-env',
    })
  })

  test('rejects a snapshot directory inside the repository', async () => {
    const fixture = await makeSnapshot()
    const inside = path.join(fixture.root, 'snapshot')
    await mkdir(inside)

    await expect(readBaselineSnapshot(inside, fixture.root)).rejects.toMatchObject({
      category: 'snapshot-env',
    })
  })

  test('rejects a metadata file whose mode is not 0600', async () => {
    const fixture = await makeSnapshot()
    await chmod(path.join(fixture.snapshot, 'pre-edit-Cargo.lock.metadata.json'), 0o644)

    await expect(readBaselineSnapshot(fixture.snapshot, fixture.root)).rejects.toMatchObject({
      category: 'snapshot-identity',
    })
  })

  test('rejects an insecure mode on a real lock snapshot', async () => {
    const fixture = await makeSnapshot()
    await chmod(fixture.lockPath, 0o644)

    await expect(readBaselineSnapshot(fixture.snapshot, fixture.root)).rejects.toMatchObject({
      category: 'snapshot-identity',
    })
  })

  test('rejects metadata special mode bits in the stat-like validator', () => {
    expect(() =>
      validateSecureRegularStat(
        {
          mode: 0o4600,
          uid: process.getuid!(),
          isFile: () => true,
        },
        'metadata file',
      ),
    ).toThrow(expect.objectContaining({ category: 'snapshot-identity' }) as PolicyError)
  })

  test('rejects lock special mode bits in the stat-like validator', () => {
    expect(() =>
      validateSecureRegularStat(
        {
          mode: 0o1600,
          uid: process.getuid!(),
          isFile: () => true,
        },
        'lock snapshot',
      ),
    ).toThrow(expect.objectContaining({ category: 'snapshot-identity' }) as PolicyError)
  })

  test('rejects same-descriptor identity changes during a read', async () => {
    const fixture = await makeSnapshot()
    const handle = await open(fixture.lockPath, 'r+')
    try {
      await expect(
        readStableFile(handle, 'lock snapshot', () => handle.chmod(0o640)),
      ).rejects.toMatchObject({ category: 'snapshot-identity' })
    } finally {
      await handle.close()
    }
  })
})

describe('diagnostic boundaries', () => {
  test('converts repository ENOENT reads without exposing the root path', async () => {
    const fixture = await makeSnapshot()

    await expect(readRepoText(fixture.root, 'missing.toml', 'root manifest')).rejects.toEqual(
      expect.objectContaining({
        category: 'repo-read',
        message: 'root manifest read failed: ENOENT',
      }),
    )
    await readRepoText(fixture.root, 'missing.toml', 'root manifest').catch(
      (error: PolicyError) => {
        expect(error.message).not.toContain(fixture.root)
      },
    )
  })

  test('converts repository EACCES reads without exposing the root path', async () => {
    const fixture = await makeSnapshot()
    const denied = path.join(fixture.root, 'denied.toml')
    await writeFile(denied, 'denied', { mode: 0o600 })
    await chmod(denied, 0o000)

    await expect(readRepoText(fixture.root, 'denied.toml', 'app manifest')).rejects.toEqual(
      expect.objectContaining({
        category: 'repo-read',
        message: 'app manifest read failed: EACCES',
      }),
    )
  })

  test('formats unexpected failures without forwarding their message', () => {
    expect(formatCliFailure(new Error(`/secret/repo: ${repoRoot}`))).toBe(
      'FAIL [internal]: internal failure\n',
    )
  })
})

const checkerPath = path.join(repoRoot, 'scripts/check-hanonly-production-policy.ts')

function baselineFromCurrentLock(current: string): string {
  let packageName = ''
  const removed: string[] = []
  const output = current
    .split('\n')
    .filter((line) => {
      if (line === '[[package]]') packageName = ''
      if (line.startsWith('name = "')) packageName = line.slice(8, -1)
      const remove =
        (packageName === 'koharu-app' && line === ' "rustix 1.1.4",') ||
        ((packageName === 'koharu-app' || packageName === 'koharu-llm') && line === ' "sha2",')
      if (remove) removed.push(`${packageName}:${line}`)
      return !remove
    })
    .join('\n')
  expect(removed).toHaveLength(3)
  return output
}

async function writeSnapshot(snapshot: string, lockBytes: Buffer): Promise<void> {
  await mkdir(snapshot)
  const lockPath = path.join(snapshot, 'pre-edit-Cargo.lock')
  await writeFile(lockPath, lockBytes, { mode: 0o600 })
  await chmod(lockPath, 0o600)
  const lockStat = await stat(lockPath)
  const metadata: SnapshotMetadata = {
    mode: '0600',
    owner_uid: process.getuid!(),
    path: 'pre-edit-Cargo.lock',
    sha256: createHash('sha256').update(lockBytes).digest('hex'),
    st_dev: lockStat.dev,
    st_ino: lockStat.ino,
    type: 'regular',
    version: metadataVersion,
  }
  const metadataPath = path.join(snapshot, 'pre-edit-Cargo.lock.metadata.json')
  await writeFile(metadataPath, JSON.stringify(metadata), { mode: 0o600 })
  await chmod(metadataPath, 0o600)
}

async function runCli(
  args: string[],
  env: Record<string, string | undefined> = {},
): Promise<{ exitCode: number; stdout: string; stderr: string }> {
  const childEnv = { ...process.env, ...env }
  for (const [key, value] of Object.entries(childEnv)) {
    if (value === undefined) delete childEnv[key]
  }
  const child = Bun.spawn([process.execPath, checkerPath, ...args], {
    cwd: repoRoot,
    env: childEnv as Record<string, string>,
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const [exitCode, stdout, stderr] = await Promise.all([
    child.exited,
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ])
  return { exitCode, stdout, stderr }
}

async function writeB0ArtifactFixture(root: string): Promise<{
  artifact: string
  b0Sha: string
  manifestSha256: string
  fixtureManifestSha256: string
  artifactSha256: string
  preCalibrationCheck: string
  preHoldoutCheck: string
}> {
  const artifact = path.join(root, 'crop-policy-selection.json')
  const b0Sha = 'b'.repeat(40)
  const manifestSha256 = 'c'.repeat(64)
  const fixtureManifestSha256 = 'd'.repeat(64)
  const checks = path.join(root, 'source-gate-selection/checks')
  await mkdir(checks, { recursive: true, mode: 0o700 })
  await chmod(path.dirname(checks), 0o700)
  await chmod(checks, 0o700)
  const preCalibrationCheck = path.join(checks, 'pre-calibration.json')
  const preHoldoutCheck = path.join(checks, 'pre-holdout.json')
  for (const [phase, output] of [
    ['pre-calibration', preCalibrationCheck],
    ['pre-holdout', preHoldoutCheck],
  ] as const) {
    const result = await runCli(['--b0-source-gate-anti-fixture'], {
      HANONLY_B0_SHA: b0Sha,
      HANONLY_B0_REQUIRED_CHECK_PHASE: phase,
      HANONLY_B0_REQUIRED_CHECK_ATTESTATION_OUT: output,
      HANONLY_EVIDENCE_ROOT: root,
      HANONLY_VISUAL_MANIFEST_SHA256: manifestSha256,
      HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256: fixtureManifestSha256,
    })
    expect(result.exitCode).toBe(0)
  }
  const child = Bun.spawn(['python3', '-', artifact, preCalibrationCheck, preHoldoutCheck], {
    cwd: repoRoot,
    stdin: 'pipe',
    stdout: 'pipe',
    stderr: 'pipe',
  })
  child.stdin.write(`
import json
import sys
from pathlib import Path
from scripts import hanonly_evidence_ledger as ledger
from scripts.hanonly_evidence_ledger_test import b0_artifact

artifact = Path(sys.argv[1])
value = b0_artifact()
checks = []
for attestation_path in map(Path, sys.argv[2:]):
    data = attestation_path.read_bytes()
    attestation = json.loads(data)
    checks.append({
        "phase": attestation["phase"],
        "command": ledger.B0_REQUIRED_CHECK_COMMAND,
        "checker_endpoint_sha256": attestation["checker_endpoint_sha256"],
        "manifest_sha256": attestation["manifest_sha256"],
        "source_gate_fixture_manifest_sha256": attestation["source_gate_fixture_manifest_sha256"],
        "attestation_relpath": attestation_path.relative_to(artifact.parent).as_posix(),
        "attestation_sha256": ledger._sha256(data),
        "b0_sha": attestation["b0_sha"],
        "result": "pass",
    })
value["required_checks"] = checks
value["frozen_payload_sha256"] = ledger._sha256(
    ledger.canonical_json(ledger._b0_frozen_projection(value))
)
raw_log_bytes = b"hanonly b0 raw log\\n"
relpaths = {
    process["load_evidence"]["raw_load_log_relpath"]
    for process in value["process_evidence"]
}
relpaths.update(
    result["execution_evidence"]["raw_inference_log_relpath"]
    for result in value["calibration_results"] + value["holdout_results"]
)
relpaths.update(
    result["execution_evidence"]["source_gate_diagnostic_relpath"]
    for result in value["calibration_results"] + value["holdout_results"]
)
for relpath in relpaths:
    path = artifact.parent / relpath
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(raw_log_bytes)
    path.chmod(0o600)
data = ledger.canonical_json(value)
artifact.write_bytes(data)
print(json.dumps({
    "b0Sha": value["b0_sha"],
    "manifestSha256": value["manifest_sha256"],
    "fixtureManifestSha256": value["source_gate_fixture_manifest_sha256"],
    "artifactSha256": ledger._sha256(data),
}))
`)
  child.stdin.end()
  const [exitCode, stdout, stderr] = await Promise.all([
    child.exited,
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ])
  expect({ exitCode, stderr }).toEqual({ exitCode: 0, stderr: '' })
  return {
    artifact,
    preCalibrationCheck,
    preHoldoutCheck,
    ...JSON.parse(stdout),
  }
}

function b0RedSources(): RustSourceFile[] {
  const b1 = [
    'hanonly_pre_b1_red_t2_dynamic_layout_contract',
    'hanonly_pre_b1_red_t2_pipeline_layout_handoff_contract',
    'hanonly_pre_b1_red_t2_source_gate_ratio_contract',
    'hanonly_pre_b1_red_t2_crop_local_ppocr_contract',
    'hanonly_pre_b1_red_t2_blob_decode_budget_contract',
    'hanonly_pre_b1_red_t2_replace_import_atomicity_contract',
    'hanonly_pre_b1_red_t2_rotation_status_contract',
  ]
  const greenC = [
    'hanonly_pre_greenc_red_t3_transient_planner_hint_contract',
    'hanonly_pre_greenc_red_t3_run_state_lifetime_contract',
    'hanonly_pre_greenc_red_t3_planner_font_outcome_contract',
    'hanonly_pre_greenc_red_t3_source_color_contract',
    'hanonly_pre_greenc_red_t3_marker_batch_atomicity_contract',
    'hanonly_pre_greenc_red_t3_untrusted_marker_lifecycle_contract',
    'hanonly_pre_greenc_red_t3_http_marker_rejection_contract',
    'hanonly_pre_greenc_red_t3_mcp_marker_rejection_contract',
    'hanonly_pre_greenc_red_t3_source_color_probe_contract',
  ]
  return [
    {
      path: 'synthetic.rs',
      text: [
        ...b1.map((id) =>
          [
            '#[test]',
            ...([
              'hanonly_pre_b1_red_t2_source_gate_ratio_contract',
              'hanonly_pre_b1_red_t2_crop_local_ppocr_contract',
            ].includes(id)
              ? []
              : ['#[ignore = "hanonly-pre-b1-red"]']),
            `fn ${id}() {}`,
          ].join('\n'),
        ),
        ...greenC.map(
          (id) => `#[tokio::test]\n#[ignore = "hanonly-pre-greenc-red"]\nasync fn ${id}() {}`,
        ),
      ].join('\n'),
    },
  ]
}

describe('RED test state policy', () => {
  test('accepts the exact B0 staged RED inventory', () => {
    expect(() => validateRedTestState(b0RedSources(), 'b0')).not.toThrow()
  })

  test('rejects B0 staged RED inventory drift', () => {
    const files = b0RedSources()
    files[0].text = files[0].text.replace(
      '#[ignore = "hanonly-pre-b1-red"]\nfn hanonly_pre_b1_red_t2_dynamic_layout_contract',
      'fn hanonly_pre_b1_red_t2_dynamic_layout_contract',
    )

    expect(() => validateRedTestState(files, 'b0')).toThrow(PolicyError)
  })

  test('rejects B0-owned source gate tests if they remain ignored', () => {
    const files = b0RedSources()
    files[0].text = files[0].text.replace(
      'fn hanonly_pre_b1_red_t2_source_gate_ratio_contract',
      '#[ignore = "hanonly-pre-b1-red"]\nfn hanonly_pre_b1_red_t2_source_gate_ratio_contract',
    )

    expect(() => validateRedTestState(files, 'b0')).toThrow(PolicyError)
  })

  test('rejects final staged RED marker residue', () => {
    expect(() => validateRedTestState(b0RedSources(), 'final')).toThrow(PolicyError)
  })
})

async function releaseFeatureSources(): Promise<RustSourceFile[]> {
  const paths = [
    'package.json',
    'ui/package.json',
    '.cargo/config.toml',
    '.github/workflows/build.yml',
    '.github/workflows/docs.yml',
    '.github/workflows/lint.yml',
    '.github/workflows/pr.yml',
    '.github/workflows/publish.yml',
    '.github/workflows/release.yml',
    '.github/workflows/test.yml',
    'crates/koharu/tauri.conf.json',
    'scripts/dev.ts',
    'scripts/release.ts',
    'Cargo.toml',
    'crates/koharu-app/Cargo.toml',
    'crates/koharu-llm/Cargo.toml',
    'crates/koharu-ml/Cargo.toml',
  ]
  return Promise.all(
    paths.map(async (relativePath) => ({
      path: relativePath,
      text: await readFile(path.join(repoRoot, relativePath), 'utf8'),
    })),
  )
}

describe('release feature inventory policy', () => {
  test('accepts the current default-off evidence feature inventory', async () => {
    const files = await releaseFeatureSources()
    expect(() => validateReleaseFeatureInventory(files)).not.toThrow()
  })

  test('rejects release surface evidence feature activation', async () => {
    const files = await releaseFeatureSources()
    files.find((file) => file.path === 'package.json')!.text += '\nhanonly-test-evidence\n'

    expect(() => validateReleaseFeatureInventory(files)).toThrow(PolicyError)
  })

  test('rejects evidence feature in a default feature list', async () => {
    const files = await releaseFeatureSources()
    const app = files.find((file) => file.path === 'crates/koharu-app/Cargo.toml')!
    app.text = app.text.replace(
      '[features]\nhanonly-test-evidence',
      '[features]\ndefault = ["hanonly-test-evidence"]\nhanonly-test-evidence',
    )

    expect(() => validateReleaseFeatureInventory(files)).toThrow(PolicyError)
  })
})

const antiFixturePaths = [
  'crates/koharu-app/src/pipeline/engines/source_language_gate.rs',
  'crates/koharu-app/src/pipeline/engines/support.rs',
  'crates/koharu-ml/src/pp_ocr_v5.rs',
  'crates/koharu-llm/src/paddleocr_vl.rs',
  'crates/koharu-app/src/pipeline/mod.rs',
  'scripts/check-hanonly-production-policy.ts',
  'scripts/check-hanonly-production-policy.test.ts',
  'scripts/hanonly_evidence_ledger.py',
  'scripts/hanonly_evidence_ledger_test.py',
] as const

async function antiFixtureSources(): Promise<RustSourceFile[]> {
  return Promise.all(
    antiFixturePaths.map(async (relativePath) => ({
      path: relativePath,
      text: await readFile(path.join(repoRoot, relativePath), 'utf8'),
    })),
  )
}

describe('B0 source gate anti-fixture policy', () => {
  test('accepts the current R51 scan roots', async () => {
    const files = await antiFixtureSources()
    expect(() => validateB0SourceGateAntiFixture(files)).not.toThrow()
  })

  test('rejects fixture branches in production Source Gate roots', async () => {
    const files = await antiFixtureSources()
    files[0].text = 'if entry_id == "h03" { return true }\n' + files[0].text

    expect(() => validateB0SourceGateAntiFixture(files)).toThrow(PolicyError)
  })

  test('rejects fixture branches in shared production support', async () => {
    const files = await antiFixtureSources()
    files[1].text = 'if entry_id == "h03" { return true }\n' + files[1].text

    expect(() => validateB0SourceGateAntiFixture(files)).toThrow(PolicyError)
  })

  test('rejects descriptor flow in production PP-OCR roots', async () => {
    const files = await antiFixtureSources()
    files[2].text = 'let source_gate_fixture_manifest_sha256 = "fixture";\n' + files[2].text

    expect(() => validateB0SourceGateAntiFixture(files)).toThrow(PolicyError)
  })

  test('writes a canonical phase-bound attestation from the CLI', async () => {
    const temporaryRoot = await realpath(await mkdtemp(path.join(os.tmpdir(), 'hanonly-r49-')))
    temporaryRoots.push(temporaryRoot)
    const evidenceRoot = path.join(temporaryRoot, 'evidence')
    await mkdir(evidenceRoot)
    const attestation = path.join(evidenceRoot, 'pre-calibration-attestation.json')

    const result = await runCli(['--b0-source-gate-anti-fixture'], {
      HANONLY_B0_SHA: 'a'.repeat(40),
      HANONLY_B0_REQUIRED_CHECK_PHASE: 'pre-calibration',
      HANONLY_B0_REQUIRED_CHECK_ATTESTATION_OUT: attestation,
      HANONLY_EVIDENCE_ROOT: evidenceRoot,
      HANONLY_VISUAL_MANIFEST_SHA256: 'b'.repeat(64),
      HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256: 'c'.repeat(64),
    })

    expect(result).toEqual({
      exitCode: 0,
      stdout: 'PASS: hanonly b0 source gate anti-fixture\n',
      stderr: '',
    })
    const parsed = JSON.parse(await readFile(attestation, 'utf8'))
    expect(Object.keys(parsed).sort()).toEqual([
      'allowed_descriptor_roots',
      'b0_sha',
      'checker_endpoint_sha256',
      'manifest_sha256',
      'mode',
      'phase',
      'policy_scan_sha256',
      'result',
      'scanned_roots',
      'source_gate_fixture_manifest_sha256',
      'version',
    ])
    expect(parsed.phase).toBe('pre-calibration')
    expect(parsed.result).toBe('pass')
    expect(parsed.scanned_roots).toEqual([...antiFixturePaths])
  })
})

function frozenInterpreterRecords(): {
  records: FrozenInterpreterRecord[]
  b0Sha: string
  implSha: string
} {
  const b0Sha = '1'.repeat(40)
  const implSha = '2'.repeat(40)
  const paths = [
    'scripts/check-hanonly-production-policy.ts',
    'scripts/check-hanonly-production-policy.test.ts',
    'scripts/hanonly_evidence_ledger.py',
    'scripts/hanonly_evidence_ledger_test.py',
    'package.json',
    'ui/package.json',
    'bun.lock',
  ]
  const records = paths.flatMap((relativePath, index) => {
    const object = `${index}`.repeat(40)
    return [
      { sha: b0Sha, path: relativePath, mode: '100644', type: 'blob', object },
      { sha: implSha, path: relativePath, mode: '100644', type: 'blob', object },
    ]
  })
  return { records, b0Sha, implSha }
}

describe('frozen interpreter policy', () => {
  test('accepts unchanged endpoint blobs for the exact frozen path set', () => {
    const fixture = frozenInterpreterRecords()
    expect(() =>
      validateFrozenInterpreterRecords(fixture.records, fixture.b0Sha, fixture.implSha),
    ).not.toThrow()
  })

  test('rejects endpoint blob drift', () => {
    const fixture = frozenInterpreterRecords()
    fixture.records[1].object = 'f'.repeat(40)

    expect(() =>
      validateFrozenInterpreterRecords(fixture.records, fixture.b0Sha, fixture.implSha),
    ).toThrow(PolicyError)
  })
})

function generatedRustFixture(): {
  sysSource: string
  defaultCargoJson: string
  evidenceCargoJson: string
  files: GeneratedRustFile[]
} {
  const names = [
    'types.rs',
    'llama_loader.rs',
    'ggml_loader.rs',
    'ggml_base_loader.rs',
    'mtmd_loader.rs',
    'wrappers.rs',
  ]
  const defaultOut = '/tmp/koharu-default-out'
  const evidenceOut = '/tmp/koharu-evidence-out'
  const cargoLine = (outDir: string) =>
    JSON.stringify({
      reason: 'build-script-executed',
      package_id: 'path+file:///repo/crates/koharu-llm#0.61.2',
      out_dir: outDir,
    })
  return {
    sysSource: names.map((name) => `include!(concat!(env!("OUT_DIR"), "/${name}"));`).join('\n'),
    defaultCargoJson: `${cargoLine(defaultOut)}\n`,
    evidenceCargoJson: `${cargoLine(evidenceOut)}\n`,
    files: [
      ...names.map((name) => ({
        label: 'default',
        path: path.join(defaultOut, name),
        text: `pub const ${name.replaceAll(/[._-]/g, '_').toUpperCase()}: usize = 1;`,
      })),
      ...names.map((name) => ({
        label: 'evidence',
        path: path.join(evidenceOut, name),
        text: `pub const ${name.replaceAll(/[._-]/g, '_').toUpperCase()}: usize = 1;`,
      })),
    ],
  }
}

async function writeGeneratedRustCliFixture(root: string): Promise<{
  defaultLog: string
  evidenceLog: string
}> {
  const names = [
    'types.rs',
    'llama_loader.rs',
    'ggml_loader.rs',
    'ggml_base_loader.rs',
    'mtmd_loader.rs',
    'wrappers.rs',
  ]
  const defaultOut = path.join(root, 'default-out')
  const evidenceOut = path.join(root, 'evidence-out')
  await Promise.all([mkdir(defaultOut), mkdir(evidenceOut)])
  for (const outDir of [defaultOut, evidenceOut]) {
    await Promise.all(
      names.map((name) => writeFile(path.join(outDir, name), `pub const OK: &str = "${name}";`)),
    )
  }
  const cargoLine = (outDir: string) =>
    `${JSON.stringify({
      reason: 'build-script-executed',
      package_id: 'path+file:///repo/crates/koharu-llm#0.61.2',
      out_dir: outDir,
    })}\n`
  const defaultLog = path.join(root, 'default.jsonl')
  const evidenceLog = path.join(root, 'evidence.jsonl')
  await Promise.all([
    writeFile(defaultLog, cargoLine(defaultOut)),
    writeFile(evidenceLog, cargoLine(evidenceOut)),
  ])
  return { defaultLog, evidenceLog }
}

describe('generated Rust policy', () => {
  test('accepts Cargo-bound generated Rust outputs', () => {
    const fixture = generatedRustFixture()
    expect(() =>
      validateGeneratedRustAudit(
        fixture.sysSource,
        fixture.defaultCargoJson,
        fixture.evidenceCargoJson,
        fixture.files,
      ),
    ).not.toThrow()
  })

  test('rejects a missing generated file', () => {
    const fixture = generatedRustFixture()
    fixture.files = fixture.files.filter((file) => file.path !== '/tmp/koharu-default-out/types.rs')

    expect(() =>
      validateGeneratedRustAudit(
        fixture.sysSource,
        fixture.defaultCargoJson,
        fixture.evidenceCargoJson,
        fixture.files,
      ),
    ).toThrow(PolicyError)
  })

  test('rejects generated corpus literals', () => {
    const fixture = generatedRustFixture()
    fixture.files[0].text += ' hanonly-test-evidence'

    expect(() =>
      validateGeneratedRustAudit(
        fixture.sysSource,
        fixture.defaultCargoJson,
        fixture.evidenceCargoJson,
        fixture.files,
      ),
    ).toThrow(PolicyError)
  })
})

describe('R51 evidence executable selection', () => {
  const artifact = {
    reason: 'compiler-artifact',
    package_id: 'path+file:///repo/crates/koharu-app#0.61.2',
    target: { kind: ['lib'], name: 'koharu_app' },
    profile: { test: true },
    features: ['hanonly-test-evidence'],
    executable: '/target/debug/deps/koharu_app-test',
  }

  test('selects exactly one evidence lib-test executable', () => {
    expect(r51EvidenceExecutable(`${JSON.stringify(artifact)}\n`)).toBe(artifact.executable)
    expect(
      r51EvidenceExecutable(
        `Storage: 33.5 GiB free, target 4.3 GiB, ui/.next 0.0 GiB\n${JSON.stringify(artifact)}\n`,
      ),
    ).toBe(artifact.executable)
  })

  test('rejects duplicate, late, and malformed storage snapshots', () => {
    const storage = 'Storage: 33.5 GiB free, target 4.3 GiB, ui/.next 0.0 GiB'
    expect(() =>
      r51EvidenceExecutable(`${storage}\n${storage}\n${JSON.stringify(artifact)}\n`),
    ).toThrow(PolicyError)
    expect(() => r51EvidenceExecutable(`${JSON.stringify(artifact)}\n${storage}\n`)).toThrow(
      PolicyError,
    )
    expect(() => r51EvidenceExecutable(`Storage: unknown\n${JSON.stringify(artifact)}\n`)).toThrow(
      PolicyError,
    )
  })

  test('uses the executable RED-state CLI contract', () => {
    expect(r51MarkerInventoryCommand).toEqual([
      'bun',
      'scripts/check-hanonly-production-policy.ts',
      '--validate-red-test-state',
      'b0',
    ])
  })

  test('rejects duplicate artifacts and feature drift', () => {
    expect(() =>
      r51EvidenceExecutable(`${JSON.stringify(artifact)}\n${JSON.stringify(artifact)}\n`),
    ).toThrow(PolicyError)
    expect(() =>
      r51EvidenceExecutable(
        `${JSON.stringify({ ...artifact, features: ['hanonly-test-evidence', 'metal'] })}\n`,
      ),
    ).toThrow(PolicyError)
    expect(() =>
      r51EvidenceExecutable(
        `${JSON.stringify({ ...artifact, target: { kind: ['lib', 'rlib'], name: 'koharu_app' } })}\n`,
      ),
    ).toThrow(PolicyError)
    expect(() =>
      r51EvidenceExecutable(
        `${JSON.stringify({ ...artifact, package_id: 'path+file:///repo/not-koharu-app#0.61.2' })}\n`,
      ),
    ).toThrow(PolicyError)
  })
})

describe('CLI contract', () => {
  test('imports without running the CLI', async () => {
    const moduleUrl = pathToFileURL(checkerPath).href
    const child = Bun.spawn([
      process.execPath,
      '-e',
      `await import(${JSON.stringify(moduleUrl)}); process.stdout.write("IMPORTED\\n")`,
    ])
    const [exitCode, stdout] = await Promise.all([child.exited, new Response(child.stdout).text()])

    expect(exitCode).toBe(0)
    expect(stdout).toBe('IMPORTED\n')
  })

  test('executes the R51 marker argv and rejects the obsolete --state form', async () => {
    const valid = await runCli(['--validate-red-test-state', 'b0'])
    expect(valid).toEqual({
      exitCode: 0,
      stdout: 'PASS: hanonly b0 red test state\n',
      stderr: '',
    })
    const obsolete = await runCli(['--validate-red-test-state', '--state', 'b0'])
    expect(obsolete.exitCode).not.toBe(0)
    expect(obsolete.stdout).toBe('')
    expect(obsolete.stderr).toStartWith('FAIL [argv]:')
  })

  test('does not start a build when the preflight custody snapshot fails', async () => {
    const temporaryRoot = await realpath(await mkdtemp(path.join(os.tmpdir(), 'hanonly-r51-')))
    temporaryRoots.push(temporaryRoot)
    const bin = path.join(temporaryRoot, 'bin')
    const spawnLog = path.join(temporaryRoot, 'spawn.log')
    const target = path.join(temporaryRoot, 'target')
    await mkdir(bin)
    await mkdir(target)
    for (const [name, exitCode] of [
      ['python3', 2],
      ['bun', 99],
    ] as const) {
      const shim = path.join(bin, name)
      await writeFile(
        shim,
        `#!${process.execPath}
import { appendFileSync } from 'node:fs'
appendFileSync(process.env.HANONLY_R51_SPAWN_LOG, ${JSON.stringify(name)} + "\\n")
process.exit(${exitCode})
`,
        { mode: 0o700 },
      )
      await chmod(shim, 0o700)
    }
    const result = await runCli(
      [
        '--write-r51-b0-preflight-attestation',
        '--output',
        path.join(temporaryRoot, 'r51-b0-preflight.json'),
        '--r51-contract',
        path.join(repoRoot, '.omx/plans/hanonly-r51-b0-custody-contract.json'),
        '--operative-plan',
        path.join(repoRoot, '.omx/plans/2026-07-23-hanonly-visual-rendering-remediation-plan.md'),
        '--r51-test-spec',
        path.join(repoRoot, '.omx/plans/test-spec-hanonly-r51-b0-custody.md'),
        '--base-production-contract',
        path.join(repoRoot, '.omx/plans/hanonly-r50-b0-evidence-contract.json'),
        '--freeze-receipt',
        path.join(temporaryRoot, 'holdout-freeze-receipt.json'),
        '--historical-inventory',
        path.join(temporaryRoot, 'historical-inventory.json'),
        '--ciphertext',
        path.join(temporaryRoot, 'holdout.enc'),
      ],
      {
        PATH: `${bin}${path.delimiter}${process.env.PATH}`,
        CARGO_TARGET_DIR: target,
        HANONLY_R51_SPAWN_LOG: spawnLog,
      },
    )
    expect(result.exitCode).not.toBe(0)
    expect(result.stdout).toBe('')
    expect(await readFile(spawnLog, 'utf8')).toBe('python3\n')
  })

  test('uses the exact cargo metadata argv and prints only PASS on success', async () => {
    const temporaryRoot = await realpath(await mkdtemp(path.join(os.tmpdir(), 'hanonly-cli-')))
    temporaryRoots.push(temporaryRoot)
    const snapshot = path.join(temporaryRoot, 'snapshot')
    const bin = path.join(temporaryRoot, 'bin')
    const argvLog = path.join(temporaryRoot, 'argv.json')
    await mkdir(bin)
    const currentLock = await readFile(path.join(repoRoot, 'Cargo.lock'), 'utf8')
    await writeSnapshot(snapshot, Buffer.from(baselineFromCurrentLock(currentLock)))
    const shim = path.join(bin, 'bun')
    await writeFile(
      shim,
      `#!${process.execPath}
import { writeFileSync } from 'node:fs'
writeFileSync(process.env.HANONLY_ARGV_LOG, JSON.stringify(process.argv.slice(2)))
const result = Bun.spawnSync({ cmd: [process.env.HANONLY_REAL_BUN, ...process.argv.slice(2)], stdin: 'inherit', stdout: 'inherit', stderr: 'inherit' })
process.exit(result.exitCode)
`,
      { mode: 0o700 },
    )
    await chmod(shim, 0o700)

    const result = await runCli(['--test-dependency-inventory'], {
      PATH: `${bin}${path.delimiter}${process.env.PATH}`,
      HANONLY_ARGV_LOG: argvLog,
      HANONLY_REAL_BUN: process.execPath,
      HANONLY_ORIGINAL_SNAPSHOT_DIR: snapshot,
    })

    expect(result).toEqual({
      exitCode: 0,
      stdout: 'PASS: hanonly production dependency inventory policy\n',
      stderr: '',
    })
    expect(JSON.parse(await readFile(argvLog, 'utf8'))).toEqual([
      '--silent',
      'run',
      'scripts/dev.ts',
      'cargo',
      'metadata',
      '--no-deps',
      '--format-version',
      '1',
    ])
  })

  test('returns a categorized path-free failure', async () => {
    const insideRepo = path.join(repoRoot, 'forbidden-snapshot')
    const result = await runCli(['--test-dependency-inventory'], {
      HANONLY_ORIGINAL_SNAPSHOT_DIR: insideRepo,
    })

    expect(result.exitCode).not.toBe(0)
    expect(result.stdout).toBe('')
    expect(result.stderr).toStartWith('FAIL [snapshot-env]:')
    expect(result.stderr).not.toContain(repoRoot)
    expect(result.stderr).not.toContain(insideRepo)
  })

  test('validates generated Rust from Cargo JSON logs', async () => {
    const temporaryRoot = await realpath(await mkdtemp(path.join(os.tmpdir(), 'hanonly-gen-')))
    temporaryRoots.push(temporaryRoot)
    const fixture = await writeGeneratedRustCliFixture(temporaryRoot)

    const result = await runCli([
      '--verify-generated-rust',
      '--cargo-default-messages',
      fixture.defaultLog,
      '--cargo-evidence-messages',
      fixture.evidenceLog,
    ])

    expect(result).toEqual({
      exitCode: 0,
      stdout: 'PASS: hanonly generated Rust audit\n',
      stderr: '',
    })
  })

  test('validates B0 authorization and emits the artifact sha256', async () => {
    const temporaryRoot = await realpath(await mkdtemp(path.join(os.tmpdir(), 'hanonly-b0-')))
    temporaryRoots.push(temporaryRoot)
    const fixture = await writeB0ArtifactFixture(temporaryRoot)

    const result = await runCli(
      [
        '--validate-b0-authorization',
        '--artifact',
        fixture.artifact,
        '--expected-b0-sha',
        fixture.b0Sha,
        '--required-check-attestation',
        fixture.preCalibrationCheck,
        '--required-check-attestation',
        fixture.preHoldoutCheck,
        '--emit-artifact-sha256',
      ],
      {
        HANONLY_VISUAL_MANIFEST_SHA256: fixture.manifestSha256,
        HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256: fixture.fixtureManifestSha256,
      },
    )

    expect(result).toEqual({ exitCode: 0, stdout: `${fixture.artifactSha256}\n`, stderr: '' })
  })

  test('rejects B0 authorization without frozen manifest environment', async () => {
    const temporaryRoot = await realpath(await mkdtemp(path.join(os.tmpdir(), 'hanonly-b0-')))
    temporaryRoots.push(temporaryRoot)
    const fixture = await writeB0ArtifactFixture(temporaryRoot)

    const result = await runCli([
      '--validate-b0-authorization',
      '--artifact',
      fixture.artifact,
      '--expected-b0-sha',
      fixture.b0Sha,
    ])

    expect(result.exitCode).not.toBe(0)
    expect(result.stdout).toBe('')
    expect(result.stderr).toStartWith('FAIL [b0-authorization]:')
  })

  test('rejects B0 authorization without both required-check attestations', async () => {
    const temporaryRoot = await realpath(await mkdtemp(path.join(os.tmpdir(), 'hanonly-b0-')))
    temporaryRoots.push(temporaryRoot)
    const fixture = await writeB0ArtifactFixture(temporaryRoot)

    const result = await runCli(
      [
        '--validate-b0-authorization',
        '--artifact',
        fixture.artifact,
        '--expected-b0-sha',
        fixture.b0Sha,
        '--required-check-attestation',
        fixture.preCalibrationCheck,
      ],
      {
        HANONLY_VISUAL_MANIFEST_SHA256: fixture.manifestSha256,
        HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256: fixture.fixtureManifestSha256,
      },
    )

    expect(result.exitCode).not.toBe(0)
    expect(result.stderr).toStartWith('FAIL [b0-authorization]:')
  })

  test('rejects B0 authorization artifact sha drift', async () => {
    const temporaryRoot = await realpath(await mkdtemp(path.join(os.tmpdir(), 'hanonly-b0-')))
    temporaryRoots.push(temporaryRoot)
    const fixture = await writeB0ArtifactFixture(temporaryRoot)

    await expect(
      validateB0Authorization(repoRoot, [
        '--validate-b0-authorization',
        '--artifact',
        fixture.artifact,
        '--expected-b0-sha',
        fixture.b0Sha,
        '--expected-artifact-sha256',
        'e'.repeat(64),
      ]),
    ).rejects.toMatchObject({ category: 'b0-authorization' })
  })
})
