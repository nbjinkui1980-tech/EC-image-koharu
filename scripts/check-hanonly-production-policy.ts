import { createHash } from 'node:crypto'
import { constants } from 'node:fs'
import type { FileHandle, Stats } from 'node:fs/promises'
import { open, readFile, realpath } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export const repoRoot = fileURLToPath(new URL('..', import.meta.url))
export const metadataVersion = 'hanonly-pre-edit-cargo-lock-metadata-v1'

const metadataName = 'pre-edit-Cargo.lock.metadata.json'
const lockName = 'pre-edit-Cargo.lock'
const metadataKeys = ['mode', 'owner_uid', 'path', 'sha256', 'st_dev', 'st_ino', 'type', 'version']
const registrySource = 'registry+https://github.com/rust-lang/crates.io-index'

export type JsonObject = Record<string, unknown>

export interface SnapshotMetadata {
  mode: '0600'
  owner_uid: number
  path: 'pre-edit-Cargo.lock'
  sha256: string
  st_dev: number
  st_ino: number
  type: 'regular'
  version: typeof metadataVersion
}

export interface DependencyInventoryInput {
  cargoMetadata: unknown
  rootManifest: unknown
  appManifest: unknown
  llmManifest: unknown
  baselineLock: unknown
  currentLock: unknown
}

export interface SecureStatLike {
  mode: number
  uid: number
  isFile(): boolean
}

export class PolicyError extends Error {
  constructor(
    public readonly category: string,
    message: string,
  ) {
    super(message)
    this.name = 'PolicyError'
  }
}

function fail(category: string, message: string): never {
  throw new PolicyError(category, message)
}

function object(value: unknown, category: string, label: string): JsonObject {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    fail(category, `${label} must be an object`)
  }
  return value as JsonObject
}

function array(value: unknown, category: string, label: string): unknown[] {
  if (!Array.isArray(value)) {
    fail(category, `${label} must be an array`)
  }
  return value
}

function currentUid(): number {
  const uid = process.getuid?.()
  if (uid === undefined) {
    fail('snapshot-identity', 'current uid is unavailable')
  }
  return uid
}

export function validateSecureRegularStat(stat: SecureStatLike, label: string): void {
  if (!stat.isFile()) {
    fail('snapshot-identity', `${label} must be a regular file`)
  }
  if (stat.uid !== currentUid()) {
    fail('snapshot-identity', `${label} owner uid mismatch`)
  }
  if ((stat.mode & 0o7777) !== 0o600) {
    fail('snapshot-identity', `${label} mode must be 0600`)
  }
}

function stableIdentity(stat: Stats): readonly number[] {
  return [
    stat.dev,
    stat.ino,
    stat.uid,
    stat.mode,
    stat.nlink,
    stat.size,
    stat.mtimeMs,
    stat.ctimeMs,
  ]
}

function equalArray(left: readonly unknown[], right: readonly unknown[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index])
}

export async function readStableFile(
  handle: FileHandle,
  label: string,
  afterRead?: () => void | Promise<void>,
): Promise<{ bytes: Buffer; stat: Stats }> {
  const before = await handle.stat()
  validateSecureRegularStat(before, label)
  const bytes = await handle.readFile()
  await afterRead?.()
  const after = await handle.stat()
  validateSecureRegularStat(after, label)
  if (!equalArray(stableIdentity(before), stableIdentity(after))) {
    fail('snapshot-identity', `${label} changed while being read`)
  }
  return { bytes, stat: after }
}

async function openNoFollow(filePath: string, missingCategory: string): Promise<FileHandle> {
  try {
    return await open(filePath, constants.O_RDONLY | constants.O_NOFOLLOW)
  } catch {
    fail(missingCategory, 'required snapshot artifact is missing or unsafe')
  }
}

function parseJson(bytes: Buffer): unknown {
  try {
    return JSON.parse(bytes.toString('utf8'))
  } catch {
    fail('snapshot-metadata', 'metadata JSON is invalid')
  }
}

export function validateSnapshotMetadata(value: unknown): SnapshotMetadata {
  const record = object(value, 'snapshot-metadata', 'metadata')
  const keys = Object.keys(record).sort()
  if (!equalArray(keys, metadataKeys)) {
    fail('snapshot-metadata', 'metadata keys are not closed and complete')
  }
  if (record.version !== metadataVersion) {
    fail('snapshot-metadata', 'metadata version mismatch')
  }
  if (record.path !== lockName) {
    fail('snapshot-metadata', 'metadata path mismatch')
  }
  if (record.mode !== '0600') {
    fail('snapshot-metadata', 'metadata mode mismatch')
  }
  if (record.type !== 'regular') {
    fail('snapshot-metadata', 'metadata type mismatch')
  }
  if (record.owner_uid !== currentUid()) {
    fail('snapshot-metadata', 'metadata owner uid mismatch')
  }
  if (!Number.isSafeInteger(record.st_dev) || !Number.isSafeInteger(record.st_ino)) {
    fail('snapshot-metadata', 'metadata device and inode must be integers')
  }
  if (typeof record.sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(record.sha256)) {
    fail('snapshot-metadata', 'metadata sha256 must be lowercase hexadecimal')
  }
  return record as unknown as SnapshotMetadata
}

export async function validateExternalSnapshotDir(
  snapshotDir: string,
  root: string,
): Promise<string> {
  if (!path.isAbsolute(snapshotDir) || path.resolve(snapshotDir) !== snapshotDir) {
    fail('snapshot-env', 'snapshot directory must be an absolute canonical path')
  }
  let canonicalSnapshot: string
  let canonicalRoot: string
  try {
    ;[canonicalSnapshot, canonicalRoot] = await Promise.all([realpath(snapshotDir), realpath(root)])
  } catch {
    fail('snapshot-env', 'snapshot directory or repository root is unavailable')
  }
  if (canonicalSnapshot !== snapshotDir) {
    fail('snapshot-env', 'snapshot directory must not traverse symlinks')
  }
  const relative = path.relative(canonicalRoot, canonicalSnapshot)
  if (
    relative === '' ||
    (relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative))
  ) {
    fail('snapshot-env', 'snapshot directory must be outside the repository')
  }
  return canonicalSnapshot
}

export async function readBaselineSnapshot(snapshotDir: string, root: string): Promise<unknown> {
  const canonicalDir = await validateExternalSnapshotDir(snapshotDir, root)
  const metadataPath = path.join(canonicalDir, metadataName)
  const lockPath = path.join(canonicalDir, lockName)
  const metadataHandle = await openNoFollow(metadataPath, 'snapshot-metadata')
  let metadata: SnapshotMetadata
  try {
    const result = await readStableFile(metadataHandle, 'metadata file')
    metadata = validateSnapshotMetadata(parseJson(result.bytes))
  } finally {
    await metadataHandle.close()
  }

  const lockHandle = await openNoFollow(lockPath, 'baseline-missing')
  try {
    const { bytes, stat } = await readStableFile(lockHandle, 'lock snapshot')
    if (stat.dev !== metadata.st_dev || stat.ino !== metadata.st_ino) {
      fail('snapshot-identity', 'lock snapshot device or inode drift')
    }
    const digest = createHash('sha256').update(bytes).digest('hex')
    if (digest !== metadata.sha256) {
      fail('snapshot-hash', 'lock snapshot sha256 mismatch')
    }
    return parseToml(bytes.toString('utf8'), 'baseline lock')
  } finally {
    await lockHandle.close()
  }
}

function parseToml(text: string, label: string): unknown {
  try {
    return Bun.TOML.parse(text)
  } catch {
    fail('parse', `${label} TOML is invalid`)
  }
}

function deepEqual(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true
  if (Array.isArray(left) && Array.isArray(right)) {
    return (
      left.length === right.length && left.every((item, index) => deepEqual(item, right[index]))
    )
  }
  if (
    left &&
    right &&
    typeof left === 'object' &&
    typeof right === 'object' &&
    !Array.isArray(left) &&
    !Array.isArray(right)
  ) {
    const leftRecord = left as JsonObject
    const rightRecord = right as JsonObject
    const leftKeys = Object.keys(leftRecord).sort()
    const rightKeys = Object.keys(rightRecord).sort()
    return (
      equalArray(leftKeys, rightKeys) &&
      leftKeys.every((key) => deepEqual(leftRecord[key], rightRecord[key]))
    )
  }
  return false
}

interface ManifestEdge {
  path: string
  key: string
  name: string
}

function dependencyNames(key: string, value: unknown): string[] {
  const names = [key]
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const packageName = (value as JsonObject).package
    if (typeof packageName === 'string' && packageName !== key) names.push(packageName)
  }
  return names
}

function relatedDependencyEntries(value: unknown, sectionPath: string): ManifestEdge[] {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return []
  const edges: ManifestEdge[] = []
  for (const [key, dependency] of Object.entries(value)) {
    for (const name of dependencyNames(key, dependency)) {
      if (name === 'rustix' || name === 'sha2') {
        edges.push({ path: sectionPath, key, name })
      }
    }
  }
  return edges
}

function relatedManifestEdges(manifest: JsonObject): ManifestEdge[] {
  const edges: ManifestEdge[] = []
  const inspect = (container: unknown, prefix: string) => {
    if (!container || typeof container !== 'object' || Array.isArray(container)) return
    const record = container as JsonObject
    for (const section of ['dependencies', 'dev-dependencies', 'build-dependencies']) {
      edges.push(...relatedDependencyEntries(record[section], `${prefix}${section}`))
    }
  }
  inspect(manifest, '')
  const targets = manifest.target
  if (targets && typeof targets === 'object' && !Array.isArray(targets)) {
    for (const [target, value] of Object.entries(targets)) inspect(value, `target.${target}.`)
  }
  return edges
}

function validateManifests(rootValue: unknown, appValue: unknown, llmValue: unknown): void {
  const root = object(rootValue, 'manifest-policy', 'root manifest')
  const app = object(appValue, 'manifest-policy', 'app manifest')
  const llm = object(llmValue, 'manifest-policy', 'llm manifest')
  const workspace = object(root.workspace, 'manifest-policy', 'root workspace')
  const workspaceDependencies = object(
    workspace.dependencies,
    'manifest-policy',
    'workspace dependencies',
  )
  const rootEdges = [
    ...relatedManifestEdges(root),
    ...relatedDependencyEntries(workspaceDependencies, 'workspace.dependencies'),
  ]
  if (rootEdges.length !== 0) {
    fail('manifest-policy', 'root workspace declares a policy dependency')
  }

  const appEdges = relatedManifestEdges(app)
  if (
    !deepEqual(appEdges, [
      { path: 'dev-dependencies', key: 'rustix', name: 'rustix' },
      { path: 'dev-dependencies', key: 'sha2', name: 'sha2' },
    ])
  ) {
    fail('manifest-policy', 'koharu-app policy dependencies are missing or misplaced')
  }
  const appDev = object(app['dev-dependencies'], 'manifest-policy', 'app dev dependencies')
  if (
    !deepEqual(appDev.rustix, { version: '=1.1.4', features: ['fs'] }) ||
    appDev.sha2 !== '=0.10.9'
  ) {
    fail('manifest-policy', 'koharu-app dependency representation drift')
  }

  const llmEdges = relatedManifestEdges(llm)
  if (!deepEqual(llmEdges, [{ path: 'build-dependencies', key: 'sha2', name: 'sha2' }])) {
    fail('manifest-policy', 'koharu-llm policy dependency is missing or misplaced')
  }
  const llmBuild = object(llm['build-dependencies'], 'manifest-policy', 'llm build dependencies')
  if (llmBuild.sha2 !== '=0.10.9') {
    fail('manifest-policy', 'koharu-llm sha2 representation drift')
  }
}

interface ExpectedEdge {
  packageName: string
  name: string
  kind: 'dev' | 'build'
  req: string
  features: string[]
}

const expectedEdges: ExpectedEdge[] = [
  { packageName: 'koharu-app', name: 'rustix', kind: 'dev', req: '=1.1.4', features: ['fs'] },
  { packageName: 'koharu-app', name: 'sha2', kind: 'dev', req: '=0.10.9', features: [] },
  { packageName: 'koharu-llm', name: 'sha2', kind: 'build', req: '=0.10.9', features: [] },
]

function validateCargoMetadata(value: unknown): void {
  const metadata = object(value, 'metadata-policy', 'cargo metadata')
  const packages = array(metadata.packages, 'metadata-policy', 'cargo metadata packages')
  const actual: Array<{ packageName: string; dependency: JsonObject }> = []
  for (const packageValue of packages) {
    const packageRecord = object(packageValue, 'metadata-policy', 'cargo package')
    const packageName = packageRecord.name
    if (typeof packageName !== 'string') fail('metadata-policy', 'cargo package name is invalid')
    for (const dependencyValue of array(
      packageRecord.dependencies,
      'metadata-policy',
      'cargo package dependencies',
    )) {
      const dependency = object(dependencyValue, 'metadata-policy', 'cargo dependency')
      if (dependency.name === 'rustix' || dependency.name === 'sha2') {
        actual.push({ packageName, dependency })
      }
    }
  }
  if (actual.length !== expectedEdges.length) {
    fail('metadata-policy', 'expected exactly three policy dependency edges')
  }
  for (const expected of expectedEdges) {
    const matches = actual.filter(
      ({ packageName, dependency }) =>
        packageName === expected.packageName &&
        dependency.name === expected.name &&
        dependency.kind === expected.kind,
    )
    if (matches.length !== 1) {
      fail(
        'metadata-policy',
        `${expected.packageName} ${expected.name} edge is missing or misplaced`,
      )
    }
    const dependency = matches[0].dependency
    if (
      dependency.req !== expected.req ||
      dependency.optional !== false ||
      dependency.target !== null ||
      dependency.rename !== null ||
      dependency.uses_default_features !== true ||
      dependency.source !== registrySource ||
      dependency.registry !== null ||
      !deepEqual(dependency.features, expected.features)
    ) {
      fail('metadata-policy', `${expected.packageName} ${expected.name} edge attributes drift`)
    }
  }
}

function lockPackages(value: unknown, label: string): JsonObject[] {
  const lock = object(value, 'lock-policy', `${label} lock`)
  return array(lock.package, 'lock-policy', `${label} packages`).map((item) =>
    object(item, 'lock-policy', `${label} package`),
  )
}

function packageKey(record: JsonObject): string {
  if (typeof record.name !== 'string' || typeof record.version !== 'string') {
    fail('lock-policy', 'lock package name or version is invalid')
  }
  if (record.source !== undefined && typeof record.source !== 'string') {
    fail('lock-policy', 'lock package source is invalid')
  }
  return JSON.stringify([record.name, record.version, record.source ?? null])
}

function packageMap(packages: JsonObject[], label: string): Map<string, JsonObject> {
  const result = new Map<string, JsonObject>()
  for (const record of packages) {
    const key = packageKey(record)
    if (result.has(key)) fail('lock-policy', `${label} lock package key is not unique`)
    result.set(key, record)
  }
  return result
}

function namedPackage(map: Map<string, JsonObject>, name: string, label: string): JsonObject {
  const matches = [...map.values()].filter((record) => record.name === name)
  if (matches.length !== 1) fail('lock-policy', `${label} must contain one ${name} record`)
  return matches[0]
}

function sortedStrings(value: unknown, label: string): string[] {
  const values = array(value, 'lock-policy', label)
  if (!values.every((item) => typeof item === 'string')) {
    fail('lock-policy', `${label} must contain strings`)
  }
  return [...(values as string[])].sort()
}

function withoutDependencies(record: JsonObject): JsonObject {
  const result = { ...record }
  delete result.dependencies
  return result
}

function validateLockInventory(baselineValue: unknown, currentValue: unknown): void {
  const baselineLock = object(baselineValue, 'lock-policy', 'baseline lock')
  const currentLock = object(currentValue, 'lock-policy', 'current lock')
  if (!deepEqual(Object.keys(baselineLock).sort(), Object.keys(currentLock).sort())) {
    fail('lock-policy', 'lock top-level structure drift')
  }
  for (const key of Object.keys(baselineLock).filter((key) => key !== 'package')) {
    if (!deepEqual(baselineLock[key], currentLock[key])) fail('lock-policy', 'lock header drift')
  }

  const baselinePackages = lockPackages(baselineLock, 'baseline')
  const currentPackages = lockPackages(currentLock, 'current')
  if (baselinePackages.length !== currentPackages.length) {
    fail('lock-policy', 'lock package count drift')
  }
  const baselineMap = packageMap(baselinePackages, 'baseline')
  const currentMap = packageMap(currentPackages, 'current')
  if (!deepEqual([...baselineMap.keys()].sort(), [...currentMap.keys()].sort())) {
    fail('lock-policy', 'lock package name, version, or source drift')
  }

  for (const [key, baselineRecord] of baselineMap) {
    const currentRecord = currentMap.get(key)!
    if (baselineRecord.name !== 'koharu-app' && baselineRecord.name !== 'koharu-llm') {
      if (!deepEqual(baselineRecord, currentRecord)) {
        fail('lock-policy', `existing ${String(baselineRecord.name)} package record drift`)
      }
    }
  }

  for (const [name, addition] of [
    ['koharu-app', ['rustix 1.1.4', 'sha2']],
    ['koharu-llm', ['sha2']],
  ] as const) {
    const baselineRecord = namedPackage(baselineMap, name, 'baseline')
    const currentRecord = namedPackage(currentMap, name, 'current')
    if (!deepEqual(withoutDependencies(baselineRecord), withoutDependencies(currentRecord))) {
      fail('lock-policy', `${name} non-dependency fields drift`)
    }
    const expectedDependencies = sortedStrings(
      baselineRecord.dependencies,
      `${name} baseline dependencies`,
    ).concat(addition)
    if (
      !deepEqual(
        expectedDependencies.sort(),
        sortedStrings(currentRecord.dependencies, `${name} current dependencies`),
      )
    ) {
      fail('lock-policy', `${name} dependency list is not the exact allowed baseline addition`)
    }
  }

  for (const [name, version] of [
    ['rustix', '1.1.4'],
    ['sha2', '0.10.9'],
  ]) {
    const key = JSON.stringify([name, version, registrySource])
    const baselineRecord = baselineMap.get(key)
    const currentRecord = currentMap.get(key)
    if (!baselineRecord || !currentRecord || !deepEqual(baselineRecord, currentRecord)) {
      fail('lock-policy', `${name} ${version} must be an unchanged baseline package record`)
    }
  }
}

export function validateDependencyInventory(input: DependencyInventoryInput): void {
  validateCargoMetadata(input.cargoMetadata)
  validateManifests(input.rootManifest, input.appManifest, input.llmManifest)
  validateLockInventory(input.baselineLock, input.currentLock)
}

export async function readRepoText(
  root: string,
  relativePath: string,
  label: string,
): Promise<string> {
  try {
    return await readFile(path.join(root, relativePath), 'utf8')
  } catch (error) {
    const code =
      error &&
      typeof error === 'object' &&
      typeof (error as NodeJS.ErrnoException).code === 'string'
        ? (error as NodeJS.ErrnoException).code!
        : 'UNKNOWN'
    fail('repo-read', `${label} read failed: ${code}`)
  }
}

function cargoMetadata(root: string): unknown {
  const result = Bun.spawnSync({
    cmd: [
      'bun',
      '--silent',
      'run',
      'scripts/dev.ts',
      'cargo',
      'metadata',
      '--no-deps',
      '--format-version',
      '1',
    ],
    cwd: root,
    stdout: 'pipe',
    stderr: 'pipe',
  })
  if (result.exitCode !== 0) {
    fail('metadata-command', `cargo metadata exited with code ${result.exitCode}`)
  }
  try {
    return JSON.parse(result.stdout.toString())
  } catch {
    fail('metadata-command', 'cargo metadata returned invalid JSON')
  }
}

export async function runDependencyInventory(root: string, snapshotDir: string): Promise<void> {
  const baselineLock = await readBaselineSnapshot(snapshotDir, root)
  const [rootText, appText, llmText, currentLockText] = await Promise.all([
    readRepoText(root, 'Cargo.toml', 'root manifest'),
    readRepoText(root, 'crates/koharu-app/Cargo.toml', 'app manifest'),
    readRepoText(root, 'crates/koharu-llm/Cargo.toml', 'llm manifest'),
    readRepoText(root, 'Cargo.lock', 'current lock'),
  ])
  validateDependencyInventory({
    cargoMetadata: cargoMetadata(root),
    rootManifest: parseToml(rootText, 'root manifest'),
    appManifest: parseToml(appText, 'app manifest'),
    llmManifest: parseToml(llmText, 'llm manifest'),
    baselineLock,
    currentLock: parseToml(currentLockText, 'current lock'),
  })
}

async function main(): Promise<void> {
  if (!deepEqual(process.argv.slice(2), ['--test-dependency-inventory'])) {
    fail('argv', 'expected exactly --test-dependency-inventory')
  }
  const snapshotDir = process.env.HANONLY_ORIGINAL_SNAPSHOT_DIR
  if (!snapshotDir) fail('snapshot-env', 'HANONLY_ORIGINAL_SNAPSHOT_DIR is required')
  await runDependencyInventory(repoRoot, snapshotDir)
  process.stdout.write('PASS: hanonly production dependency inventory policy\n')
}

export function formatCliFailure(error: unknown): string {
  if (error instanceof PolicyError) {
    return `FAIL [${error.category}]: ${error.message}\n`
  }
  return 'FAIL [internal]: internal failure\n'
}

if (import.meta.main) {
  main().catch((error: unknown) => {
    process.stderr.write(formatCliFailure(error))
    process.exitCode = 1
  })
}
