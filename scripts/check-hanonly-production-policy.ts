import { createHash } from 'node:crypto'
import { constants } from 'node:fs'
import type { FileHandle, Stats } from 'node:fs/promises'
import { mkdir, open, readFile, realpath, stat } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export const repoRoot = fileURLToPath(new URL('..', import.meta.url))
export const metadataVersion = 'hanonly-pre-edit-cargo-lock-metadata-v1'

const metadataName = 'pre-edit-Cargo.lock.metadata.json'
const lockName = 'pre-edit-Cargo.lock'
const metadataKeys = ['mode', 'owner_uid', 'path', 'sha256', 'st_dev', 'st_ino', 'type', 'version']
const registrySource = 'registry+https://github.com/rust-lang/crates.io-index'
const hex40 = /^[0-9a-f]{40}$/
const hex64 = /^[0-9a-f]{64}$/

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

export interface RustSourceFile {
  path: string
  text: string
}

export interface FrozenInterpreterRecord {
  sha: string
  path: string
  mode: string
  type: string
  object: string
}

export interface GeneratedRustFile {
  label: string
  path: string
  text: string
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

function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`
  if (value && typeof value === 'object') {
    return `{${Object.keys(value as JsonObject)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalJson((value as JsonObject)[key])}`)
      .join(',')}}`
  }
  return JSON.stringify(value)
}

function requiredArg(args: readonly string[], name: string): string {
  const index = args.indexOf(name)
  if (index === -1 || index + 1 >= args.length || args[index + 1].startsWith('--')) {
    fail('argv', `${name} is required`)
  }
  return args[index + 1]
}

function optionalArg(args: readonly string[], name: string): string | undefined {
  const index = args.indexOf(name)
  if (index === -1) return undefined
  if (index + 1 >= args.length || args[index + 1].startsWith('--')) {
    fail('argv', `${name} requires a value`)
  }
  return args[index + 1]
}

function repeatedArgs(args: readonly string[], name: string): string[] {
  const values: string[] = []
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] !== name) continue
    if (index + 1 >= args.length || args[index + 1].startsWith('--')) {
      fail('argv', `${name} requires a value`)
    }
    values.push(args[index + 1])
  }
  return values
}

function flag(args: readonly string[], name: string): boolean {
  return args.includes(name)
}

const stagedRedMarkers = ['hanonly-pre-b1-red', 'hanonly-pre-greenc-red'] as const
const expectedB1RedIds = [
  'hanonly_pre_b1_red_t2_dynamic_layout_contract',
  'hanonly_pre_b1_red_t2_pipeline_layout_handoff_contract',
  'hanonly_pre_b1_red_t2_source_gate_ratio_contract',
  'hanonly_pre_b1_red_t2_crop_local_ppocr_contract',
  'hanonly_pre_b1_red_t2_blob_decode_budget_contract',
  'hanonly_pre_b1_red_t2_replace_import_atomicity_contract',
  'hanonly_pre_b1_red_t2_rotation_status_contract',
] as const
const b0OwnedRedIds = [
  'hanonly_pre_b1_red_t2_source_gate_ratio_contract',
  'hanonly_pre_b1_red_t2_crop_local_ppocr_contract',
] as const
const expectedB0B1MarkerIds = expectedB1RedIds.filter(
  (id) => !(b0OwnedRedIds as readonly string[]).includes(id),
)
const expectedGreenCRedIds = [
  'hanonly_pre_greenc_red_t3_transient_planner_hint_contract',
  'hanonly_pre_greenc_red_t3_run_state_lifetime_contract',
  'hanonly_pre_greenc_red_t3_planner_font_outcome_contract',
  'hanonly_pre_greenc_red_t3_source_color_contract',
  'hanonly_pre_greenc_red_t3_marker_batch_atomicity_contract',
  'hanonly_pre_greenc_red_t3_untrusted_marker_lifecycle_contract',
  'hanonly_pre_greenc_red_t3_http_marker_rejection_contract',
  'hanonly_pre_greenc_red_t3_mcp_marker_rejection_contract',
  'hanonly_pre_greenc_red_t3_source_color_probe_contract',
] as const
const expectedRedByMarker = new Map<string, readonly string[]>([
  ['hanonly-pre-b1-red', expectedB1RedIds],
  ['hanonly-pre-greenc-red', expectedGreenCRedIds],
])
const releaseFeatureForbiddenNeedles = [
  'hanonly-test-evidence',
  '--all-features',
  'CARGO_FEATURE_HANONLY_TEST_EVIDENCE',
] as const
const releaseFeatureSurfacePaths = [
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
] as const
const featureManifestPaths = [
  'Cargo.toml',
  'crates/koharu-app/Cargo.toml',
  'crates/koharu-llm/Cargo.toml',
  'crates/koharu-ml/Cargo.toml',
] as const
const frozenInterpreterPaths = [
  'scripts/check-hanonly-production-policy.ts',
  'scripts/check-hanonly-production-policy.test.ts',
  'scripts/hanonly_evidence_ledger.py',
  'scripts/hanonly_evidence_ledger_test.py',
  'package.json',
  'ui/package.json',
  'bun.lock',
] as const
const generatedRustFiles = [
  'types.rs',
  'llama_loader.rs',
  'ggml_loader.rs',
  'ggml_base_loader.rs',
  'mtmd_loader.rs',
  'wrappers.rs',
] as const
const generatedRustForbiddenNeedles = [
  'hanonly',
  'HANONLY',
  'typographyPlanVerified',
  'crop-policy-selection',
  'test.jpeg',
] as const
const b0AntiFixtureProductionRoots = [
  'crates/koharu-app/src/pipeline/engines/source_language_gate.rs',
  'crates/koharu-ml/src/pp_ocr_v5.rs',
  'crates/koharu-llm/src/paddleocr_vl.rs',
] as const
const b0AntiFixtureEvidenceRoots = ['crates/koharu-app/src/pipeline/mod.rs'] as const
const b0AntiFixtureScriptRoots = [
  'scripts/check-hanonly-production-policy.ts',
  'scripts/check-hanonly-production-policy.test.ts',
  'scripts/hanonly_evidence_ledger.py',
  'scripts/hanonly_evidence_ledger_test.py',
] as const
const b0AntiFixtureScannedRoots = [
  ...b0AntiFixtureProductionRoots,
  ...b0AntiFixtureEvidenceRoots,
  ...b0AntiFixtureScriptRoots,
] as const
const b0AntiFixtureAllowedDescriptorRoots = [
  ...b0AntiFixtureEvidenceRoots,
  ...b0AntiFixtureScriptRoots,
] as const

type B0AntiFixturePhase = 'pre-calibration' | 'pre-holdout'

interface B0AntiFixtureVerdict {
  category: string
  root: string
  result: 'pass'
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

function sourceByPath(files: readonly RustSourceFile[], relativePath: string): RustSourceFile {
  const file = files.find((item) => item.path === relativePath)
  if (!file) fail('b0-source-gate-anti-fixture', `${relativePath} is missing`)
  return file
}

function productionRustText(file: RustSourceFile): string {
  return file.text.split(/\n#\[cfg\(test\)\]\s*\nmod tests\b/, 1)[0]
}

function b0AntiFixtureForbiddenVerdicts(files: readonly RustSourceFile[]): B0AntiFixtureVerdict[] {
  const checks: Array<{ category: string; pattern: RegExp }> = [
    {
      category: 'fixture_name',
      pattern:
        /\b(?:c0[1-4]|h0[1-4])\b|test\.(?:jpe?g|webp)|peach-hip|s-curve|full-body-shaping|slim-waist|confidence-body/,
    },
    { category: 'fixed_hash', pattern: /\b[0-9a-f]{64}\b/i },
    {
      category: 'fixed_dimension',
      pattern: /\b(?:width|height|image_width|image_height)\s*(?:==|!=)\s*\d{2,5}/,
    },
    {
      category: 'fixed_crop',
      pattern: /\[\s*\d{1,5}\s*,\s*\d{1,5}\s*,\s*\d{1,5}\s*,\s*\d{1,5}\s*\]/,
    },
    {
      category: 'fixed_node_id',
      pattern: /NodeId::(?:from|from_u128|from_bytes)|node_id\s*(?:==|!=)\s*["']/,
    },
    {
      category: 'corpus_role',
      pattern:
        /corpus_role|entry_role|role\s*(?:==|!=)\s*["'](?:calibration|holdout|regression)["']/,
    },
  ]
  const verdicts: B0AntiFixtureVerdict[] = []
  for (const root of b0AntiFixtureProductionRoots) {
    const text = productionRustText(sourceByPath(files, root))
    for (const check of checks) {
      if (check.pattern.test(text)) {
        fail('b0-source-gate-anti-fixture', `${root} contains ${check.category}`)
      }
      verdicts.push({ category: check.category, root, result: 'pass' })
    }
  }
  return verdicts
}

function b0AntiFixtureDescriptorVerdicts(files: readonly RustSourceFile[]): B0AntiFixtureVerdict[] {
  const descriptorPattern =
    /source_gate_fixture_manifest_sha256|manifest_sha256|fixtureManifestSha256|EntryRole|VisualManifest/
  const verdicts: B0AntiFixtureVerdict[] = []
  for (const root of b0AntiFixtureProductionRoots) {
    if (descriptorPattern.test(productionRustText(sourceByPath(files, root)))) {
      fail('b0-source-gate-anti-fixture', `${root} consumes descriptor data`)
    }
    verdicts.push({ category: 'descriptor_data_absent_from_production', root, result: 'pass' })
  }
  for (const root of b0AntiFixtureAllowedDescriptorRoots) {
    sourceByPath(files, root)
    verdicts.push({ category: 'descriptor_data_allowed_root', root, result: 'pass' })
  }
  return verdicts
}

function validateB0SourceGateTraceability(
  files: readonly RustSourceFile[],
): B0AntiFixtureVerdict[] {
  const sourceGate = sourceByPath(
    files,
    'crates/koharu-app/src/pipeline/engines/source_language_gate.rs',
  ).text
  for (const needle of [
    'select_chinese_target_with_fallback',
    'validate_pp_vl_alignment_internal',
    'safe_crop_bounds_with_policy',
    'crop_policy_parameters',
    'compute_safe_crop_bounds',
    'SourceGateDecision::Accepted',
  ]) {
    if (!sourceGate.includes(needle)) {
      fail('b0-source-gate-anti-fixture', `Source Gate traceability lost ${needle}`)
    }
  }
  const ppOcr = sourceByPath(files, 'crates/koharu-ml/src/pp_ocr_v5.rs').text
  for (const needle of ['word_box_inference_scale', 'word_box_source_bbox', 'pub fn word_boxes']) {
    if (!ppOcr.includes(needle)) {
      fail('b0-source-gate-anti-fixture', `PP-OCR traceability lost ${needle}`)
    }
  }
  return [
    {
      category: 'source_gate_acceptance_traces_to_ocr_vl_geometry',
      root: 'crates/koharu-app/src/pipeline/engines/source_language_gate.rs',
      result: 'pass',
    },
    {
      category: 'ppocr_crop_local_scaling_traceable',
      root: 'crates/koharu-ml/src/pp_ocr_v5.rs',
      result: 'pass',
    },
  ]
}

export function validateB0SourceGateAntiFixture(
  files: readonly RustSourceFile[],
): B0AntiFixtureVerdict[] {
  const actualRoots = files.map((file) => file.path)
  if (!deepEqual(actualRoots, [...b0AntiFixtureScannedRoots])) {
    fail('b0-source-gate-anti-fixture', 'scanned roots drift')
  }
  return [
    ...b0AntiFixtureForbiddenVerdicts(files),
    ...b0AntiFixtureDescriptorVerdicts(files),
    ...validateB0SourceGateTraceability(files),
  ]
}

async function readB0AntiFixtureSources(root: string): Promise<RustSourceFile[]> {
  return Promise.all(
    b0AntiFixtureScannedRoots.map(async (relativePath) => ({
      path: relativePath,
      text: await readRepoText(root, relativePath, relativePath),
    })),
  )
}

function requiredHashEnv(name: string): string {
  const value = process.env[name]
  if (!value || !hex64.test(value)) {
    fail('b0-source-gate-anti-fixture', `${name} is required`)
  }
  return value
}

function requiredB0ShaEnv(): string {
  const value = process.env.HANONLY_B0_SHA
  if (!value || !hex40.test(value)) {
    fail('b0-source-gate-anti-fixture', 'HANONLY_B0_SHA is required')
  }
  return value
}

function requiredB0AntiFixturePhase(): B0AntiFixturePhase {
  const value = process.env.HANONLY_B0_REQUIRED_CHECK_PHASE
  if (value !== 'pre-calibration' && value !== 'pre-holdout') {
    fail('b0-source-gate-anti-fixture', 'HANONLY_B0_REQUIRED_CHECK_PHASE is required')
  }
  return value
}

async function b0AntiFixtureOutputPath(root: string): Promise<string> {
  const output = process.env.HANONLY_B0_REQUIRED_CHECK_ATTESTATION_OUT
  if (!output || !path.isAbsolute(output) || path.resolve(output) !== output) {
    fail('b0-source-gate-anti-fixture', 'HANONLY_B0_REQUIRED_CHECK_ATTESTATION_OUT is required')
  }
  const parent = path.dirname(output)
  let canonicalParent: string
  try {
    canonicalParent = await realpath(parent)
  } catch {
    fail('b0-source-gate-anti-fixture', 'attestation parent is unavailable')
  }
  if (parent !== canonicalParent || path.basename(output).includes(path.sep)) {
    fail('b0-source-gate-anti-fixture', 'attestation output path is not canonical')
  }
  const evidenceRoot = process.env.HANONLY_EVIDENCE_ROOT
  if (evidenceRoot) {
    let canonicalEvidenceRoot: string
    try {
      canonicalEvidenceRoot = await realpath(evidenceRoot)
    } catch {
      fail('b0-source-gate-anti-fixture', 'HANONLY_EVIDENCE_ROOT is unavailable')
    }
    const relative = path.relative(canonicalEvidenceRoot, output)
    if (relative === '' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
      fail('b0-source-gate-anti-fixture', 'attestation output is outside evidence root')
    }
  }
  const repoRelative = path.relative(await realpath(root), output)
  if (
    repoRelative === '' ||
    (!repoRelative.startsWith(`..${path.sep}`) && !path.isAbsolute(repoRelative))
  ) {
    fail('b0-source-gate-anti-fixture', 'attestation output must be outside the repository')
  }
  return output
}

async function writeSyncedFile(filePath: string, data: string): Promise<void> {
  const handle = await open(
    filePath,
    constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY,
    0o600,
  )
  try {
    await handle.writeFile(data)
    await handle.sync()
  } finally {
    await handle.close()
  }
  const parent = await open(path.dirname(filePath), constants.O_RDONLY)
  try {
    await parent.sync()
  } finally {
    await parent.close()
  }
}

export async function runB0SourceGateAntiFixture(root: string): Promise<void> {
  const phase = requiredB0AntiFixturePhase()
  const b0Sha = requiredB0ShaEnv()
  const manifestSha256 = requiredHashEnv('HANONLY_VISUAL_MANIFEST_SHA256')
  const fixtureManifestSha256 = requiredHashEnv('HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256')
  const output = await b0AntiFixtureOutputPath(root)
  const attestation = await buildB0SourceGateAntiFixtureAttestation(
    root,
    phase,
    b0Sha,
    manifestSha256,
    fixtureManifestSha256,
  )
  await writeSyncedFile(output, `${canonicalJson(attestation)}\n`)
}

async function scanB0SourceGateAntiFixture(root: string): Promise<void> {
  validateB0SourceGateAntiFixture(await readB0AntiFixtureSources(root))
}

async function buildB0SourceGateAntiFixtureAttestation(
  root: string,
  phase: B0AntiFixturePhase,
  b0Sha: string,
  manifestSha256: string,
  fixtureManifestSha256: string,
): Promise<JsonObject> {
  const files = await readB0AntiFixtureSources(root)
  const verdicts = validateB0SourceGateAntiFixture(files)
  const checkerEndpointSha256 = createHash('sha256')
    .update(sourceByPath(files, 'scripts/check-hanonly-production-policy.ts').text)
    .digest('hex')
  const scan = {
    version: 1,
    mode: 'b0-source-gate-anti-fixture',
    phase,
    b0_sha: b0Sha,
    manifest_sha256: manifestSha256,
    source_gate_fixture_manifest_sha256: fixtureManifestSha256,
    checker_endpoint_sha256: checkerEndpointSha256,
    scanned_roots: [...b0AntiFixtureScannedRoots],
    allowed_descriptor_roots: [...b0AntiFixtureAllowedDescriptorRoots],
    forbidden_category_verdicts: verdicts.filter(
      (verdict) => verdict.category !== 'descriptor_data_allowed_root',
    ),
    descriptor_use_verdicts: verdicts.filter((verdict) =>
      verdict.category.startsWith('descriptor_data'),
    ),
  }
  return {
    version: 1,
    mode: 'b0-source-gate-anti-fixture',
    phase,
    b0_sha: b0Sha,
    manifest_sha256: manifestSha256,
    source_gate_fixture_manifest_sha256: fixtureManifestSha256,
    checker_endpoint_sha256: checkerEndpointSha256,
    scanned_roots: [...b0AntiFixtureScannedRoots],
    allowed_descriptor_roots: [...b0AntiFixtureAllowedDescriptorRoots],
    policy_scan_sha256: createHash('sha256').update(canonicalJson(scan)).digest('hex'),
    result: 'pass',
  }
}

function trackedRustPaths(root: string): string[] {
  const result = Bun.spawnSync({
    cmd: ['git', 'ls-files', '*.rs'],
    cwd: root,
    stdout: 'pipe',
    stderr: 'pipe',
  })
  if (result.exitCode !== 0) {
    fail('red-test-state', `git ls-files exited with code ${result.exitCode}`)
  }
  return result.stdout
    .toString()
    .split('\n')
    .filter((line) => line.length > 0)
}

export async function readTrackedRustSources(root: string): Promise<RustSourceFile[]> {
  const paths = trackedRustPaths(root)
  return Promise.all(
    paths.map(async (relativePath) => ({
      path: relativePath,
      text: await readRepoText(root, relativePath, relativePath),
    })),
  )
}

function countOccurrences(text: string, needle: string): number {
  let count = 0
  let offset = 0
  while (true) {
    const index = text.indexOf(needle, offset)
    if (index === -1) return count
    count += 1
    offset = index + needle.length
  }
}

function regexEscape(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

function countTestFunctions(text: string, id: string): number {
  return [
    ...text.matchAll(new RegExp(`(?:^|\\n)\\s*(?:async\\s+)?fn\\s+${regexEscape(id)}\\s*\\(`, 'g')),
  ].length
}

function stagedRedEntries(files: readonly RustSourceFile[]): Map<string, string[]> {
  const result = new Map<string, string[]>()
  const markerPattern =
    /#\[\s*ignore\s*=\s*"([^"]+)"\s*\]\s*(?:\r?\n\s*#\[[^\]]+\]\s*)*\r?\n\s*(?:async\s+)?fn\s+([A-Za-z0-9_]+)\s*\(/g
  for (const file of files) {
    for (const match of file.text.matchAll(markerPattern)) {
      const marker = match[1]
      if (!stagedRedMarkers.includes(marker as (typeof stagedRedMarkers)[number])) continue
      const id = match[2]
      const entries = result.get(marker) ?? []
      entries.push(id)
      result.set(marker, entries)
    }
  }
  return result
}

export function validateRedTestState(
  files: readonly RustSourceFile[],
  state: 'b0' | 'final',
): void {
  const fullText = files.map((file) => file.text).join('\n')
  for (const marker of stagedRedMarkers) {
    const expectedIds = expectedRedByMarker.get(marker)!
    if (state === 'b0') {
      const expectedMarkerIds =
        marker === 'hanonly-pre-b1-red' ? expectedB0B1MarkerIds : expectedIds
      const actualIds = [...(stagedRedEntries(files).get(marker) ?? [])].sort()
      if (!deepEqual(actualIds, [...expectedMarkerIds].sort())) {
        fail('red-test-state', `${marker} inventory drift`)
      }
    } else if (countOccurrences(fullText, marker) !== 0) {
      fail('red-test-state', `${marker} marker remains after final`)
    }
    for (const id of expectedIds) {
      if (countTestFunctions(fullText, id) !== 1) {
        fail('red-test-state', `${id} must exist exactly once`)
      }
    }
  }
}

export async function runRedTestState(root: string, state: 'b0' | 'final'): Promise<void> {
  validateRedTestState(await readTrackedRustSources(root), state)
}

function fileByPath(files: readonly RustSourceFile[], relativePath: string): RustSourceFile {
  const file = files.find((item) => item.path === relativePath)
  if (!file) fail('release-feature-inventory', `${relativePath} is missing`)
  return file
}

function featureList(value: unknown, label: string): unknown[] {
  if (value === undefined) return []
  return array(value, 'release-feature-inventory', label)
}

function validateDefaultFeatureAbsence(manifest: JsonObject, label: string): void {
  const features = manifest.features
  if (features === undefined) return
  const featureRecord = object(features, 'release-feature-inventory', `${label} features`)
  if (
    featureList(featureRecord.default, `${label} default feature`).includes('hanonly-test-evidence')
  ) {
    fail('release-feature-inventory', `${label} default enables hanonly-test-evidence`)
  }
}

export function validateReleaseFeatureInventory(files: readonly RustSourceFile[]): void {
  for (const relativePath of releaseFeatureSurfacePaths) {
    const file = fileByPath(files, relativePath)
    for (const needle of releaseFeatureForbiddenNeedles) {
      if (file.text.includes(needle)) {
        fail('release-feature-inventory', `${relativePath} contains forbidden feature activation`)
      }
    }
  }

  const root = object(
    parseToml(fileByPath(files, 'Cargo.toml').text, 'root manifest'),
    'release-feature-inventory',
    'root manifest',
  )
  validateDefaultFeatureAbsence(root, 'root manifest')

  const app = object(
    parseToml(fileByPath(files, 'crates/koharu-app/Cargo.toml').text, 'app manifest'),
    'release-feature-inventory',
    'app manifest',
  )
  const appFeatures = object(app.features, 'release-feature-inventory', 'app features')
  if (
    !deepEqual(appFeatures['hanonly-test-evidence'], [
      'koharu-llm/hanonly-test-evidence',
      'koharu-ml/hanonly-test-evidence',
    ])
  ) {
    fail('release-feature-inventory', 'app evidence feature propagation drift')
  }
  validateDefaultFeatureAbsence(app, 'app manifest')

  for (const [relativePath, label] of [
    ['crates/koharu-llm/Cargo.toml', 'llm manifest'],
    ['crates/koharu-ml/Cargo.toml', 'ml manifest'],
  ] as const) {
    const manifest = object(
      parseToml(fileByPath(files, relativePath).text, label),
      'release-feature-inventory',
      label,
    )
    const features = object(manifest.features, 'release-feature-inventory', `${label} features`)
    if (!deepEqual(features['hanonly-test-evidence'], [])) {
      fail('release-feature-inventory', `${label} evidence feature declaration drift`)
    }
    validateDefaultFeatureAbsence(manifest, label)
  }
}

export async function runReleaseFeatureInventory(root: string): Promise<void> {
  const files = await Promise.all(
    [...releaseFeatureSurfacePaths, ...featureManifestPaths].map(async (relativePath) => ({
      path: relativePath,
      text: await readRepoText(root, relativePath, relativePath),
    })),
  )
  validateReleaseFeatureInventory(files)
}

function parseLsTreeLine(sha: string, line: string): FrozenInterpreterRecord {
  const match = /^([0-7]{6}) (blob|tree|commit) ([0-9a-f]{40})\t(.+)$/.exec(line)
  if (!match) fail('frozen-interpreter', 'git ls-tree output drift')
  return {
    sha,
    mode: match[1],
    type: match[2],
    object: match[3],
    path: match[4],
  }
}

function lsTreeFrozenInterpreter(root: string, sha: string): FrozenInterpreterRecord[] {
  const result = Bun.spawnSync({
    cmd: ['git', 'ls-tree', sha, '--', ...frozenInterpreterPaths],
    cwd: root,
    stdout: 'pipe',
    stderr: 'pipe',
  })
  if (result.exitCode !== 0) {
    fail('frozen-interpreter', `git ls-tree exited with code ${result.exitCode}`)
  }
  return result.stdout
    .toString()
    .split('\n')
    .filter((line) => line.length > 0)
    .map((line) => parseLsTreeLine(sha, line))
}

function recordsByPath(
  records: readonly FrozenInterpreterRecord[],
  sha: string,
): Map<string, FrozenInterpreterRecord> {
  const result = new Map<string, FrozenInterpreterRecord>()
  for (const record of records.filter((item) => item.sha === sha)) {
    if (result.has(record.path)) fail('frozen-interpreter', `${sha} duplicate frozen path`)
    result.set(record.path, record)
  }
  return result
}

export function validateFrozenInterpreterRecords(
  records: readonly FrozenInterpreterRecord[],
  b0Sha: string,
  implSha: string,
): void {
  if (!hex40.test(b0Sha) || !hex40.test(implSha)) {
    fail('frozen-interpreter', 'endpoint sha must be 40 lowercase hexadecimal characters')
  }
  const b0 = recordsByPath(records, b0Sha)
  const impl = recordsByPath(records, implSha)
  for (const relativePath of frozenInterpreterPaths) {
    const left = b0.get(relativePath)
    const right = impl.get(relativePath)
    if (!left || !right) fail('frozen-interpreter', `${relativePath} missing at endpoint`)
    if (left.mode !== '100644' || right.mode !== '100644') {
      fail('frozen-interpreter', `${relativePath} mode drift`)
    }
    if (left.type !== 'blob' || right.type !== 'blob') {
      fail('frozen-interpreter', `${relativePath} type drift`)
    }
    if (left.object !== right.object) {
      fail('frozen-interpreter', `${relativePath} blob drift`)
    }
  }
  for (const record of records) {
    if (!(frozenInterpreterPaths as readonly string[]).includes(record.path)) {
      fail('frozen-interpreter', 'unexpected frozen interpreter path')
    }
  }
}

export function runFrozenInterpreterCheck(root: string, b0Sha: string, implSha: string): void {
  const b0Records = lsTreeFrozenInterpreter(root, b0Sha)
  validateFrozenInterpreterRecords(
    b0Sha === implSha ? b0Records : [...b0Records, ...lsTreeFrozenInterpreter(root, implSha)],
    b0Sha,
    implSha,
  )
}

function parseCargoJsonLines(text: string, label: string): JsonObject[] {
  return text
    .split('\n')
    .filter((line) => line.trim().length > 0)
    .map((line) => object(parseJson(Buffer.from(line)), 'generated-rust', `${label} cargo line`))
}

function generatedOutDirFromCargoJson(text: string, label: string): string {
  const matches = parseCargoJsonLines(text, label).filter(
    (record) =>
      record.reason === 'build-script-executed' &&
      typeof record.package_id === 'string' &&
      record.package_id.includes('koharu-llm') &&
      typeof record.out_dir === 'string',
  )
  if (matches.length !== 1) {
    fail('generated-rust', `${label} must contain exactly one koharu-llm build-script output`)
  }
  return matches[0].out_dir as string
}

function validateGeneratedSysIncludes(sysSource: string): void {
  const includes = [
    ...sysSource.matchAll(/include!\(concat!\(env!\("OUT_DIR"\), "\/([^"]+)"\)\);/g),
  ]
    .map((match) => match[1])
    .sort()
  if (!deepEqual(includes, [...generatedRustFiles].sort())) {
    fail('generated-rust', 'production OUT_DIR include set drift')
  }
}

function validateGeneratedRustFiles(files: readonly GeneratedRustFile[]): void {
  const expected = new Set(generatedRustFiles)
  const labels = new Set(files.map((file) => file.label))
  for (const label of labels) {
    const names = files
      .filter((file) => file.label === label)
      .map((file) => path.basename(file.path))
      .sort()
    if (!deepEqual(names, [...expected].sort())) {
      fail('generated-rust', `${label} generated file set drift`)
    }
  }
  for (const file of files) {
    if (!expected.has(path.basename(file.path) as (typeof generatedRustFiles)[number])) {
      fail('generated-rust', 'unexpected generated Rust file')
    }
    if (file.text.length === 0) fail('generated-rust', `${file.label} generated file is empty`)
    for (const needle of generatedRustForbiddenNeedles) {
      if (file.text.includes(needle)) {
        fail('generated-rust', `${file.label} generated file contains forbidden corpus literal`)
      }
    }
  }
}

export function validateGeneratedRustAudit(
  sysSource: string,
  defaultCargoJson: string,
  evidenceCargoJson: string,
  generatedFiles: readonly GeneratedRustFile[],
): void {
  validateGeneratedSysIncludes(sysSource)
  const expectedDirs = new Set([
    generatedOutDirFromCargoJson(defaultCargoJson, 'default'),
    generatedOutDirFromCargoJson(evidenceCargoJson, 'evidence'),
  ])
  for (const file of generatedFiles) {
    if (!expectedDirs.has(path.dirname(file.path))) {
      fail('generated-rust', 'generated file path is not bound to Cargo out_dir')
    }
  }
  validateGeneratedRustFiles(generatedFiles)
}

async function readGeneratedRustFromOutDir(
  label: string,
  outDir: string,
): Promise<GeneratedRustFile[]> {
  if (!path.isAbsolute(outDir) || (await realpath(outDir)) !== outDir) {
    fail('generated-rust', `${label} out_dir must be absolute and canonical`)
  }
  return Promise.all(
    generatedRustFiles.map(async (fileName) => {
      const filePath = path.join(outDir, fileName)
      return { label, path: filePath, text: await readFile(filePath, 'utf8') }
    }),
  )
}

export async function runGeneratedRustAudit(
  root: string,
  defaultCargoJsonPath: string,
  evidenceCargoJsonPath: string,
): Promise<void> {
  const [sysSource, defaultCargoJson, evidenceCargoJson] = await Promise.all([
    readRepoText(root, 'crates/koharu-llm/src/sys/mod.rs', 'llm sys module'),
    readFile(defaultCargoJsonPath, 'utf8'),
    readFile(evidenceCargoJsonPath, 'utf8'),
  ])
  const [defaultFiles, evidenceFiles] = await Promise.all([
    readGeneratedRustFromOutDir(
      'default',
      generatedOutDirFromCargoJson(defaultCargoJson, 'default'),
    ),
    readGeneratedRustFromOutDir(
      'evidence',
      generatedOutDirFromCargoJson(evidenceCargoJson, 'evidence'),
    ),
  ])
  validateGeneratedRustAudit(sysSource, defaultCargoJson, evidenceCargoJson, [
    ...defaultFiles,
    ...evidenceFiles,
  ])
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

export async function validateB0Authorization(
  root: string,
  args: readonly string[],
): Promise<string | undefined> {
  const artifact = requiredArg(args, '--artifact')
  const expectedB0Sha = requiredArg(args, '--expected-b0-sha')
  const expectedArtifactSha256 = optionalArg(args, '--expected-artifact-sha256')
  if (!path.isAbsolute(artifact) || (await realpath(artifact)) !== artifact) {
    fail('b0-authorization', 'artifact path must be absolute and canonical')
  }
  if (!hex40.test(expectedB0Sha)) {
    fail('b0-authorization', 'expected B0 sha must be 40 lowercase hexadecimal characters')
  }
  if (expectedArtifactSha256 !== undefined && !hex64.test(expectedArtifactSha256)) {
    fail('b0-authorization', 'expected artifact sha256 must be lowercase hexadecimal')
  }
  const visualManifestSha256 = process.env.HANONLY_VISUAL_MANIFEST_SHA256
  const fixtureManifestSha256 = process.env.HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256
  if (!visualManifestSha256 || !hex64.test(visualManifestSha256)) {
    fail('b0-authorization', 'HANONLY_VISUAL_MANIFEST_SHA256 is required')
  }
  if (!fixtureManifestSha256 || !hex64.test(fixtureManifestSha256)) {
    fail('b0-authorization', 'HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256 is required')
  }
  const requiredCheckAttestations = repeatedArgs(args, '--required-check-attestation')
  if (requiredCheckAttestations.length !== 2) {
    fail('b0-authorization', 'exactly two required-check attestations are required')
  }

  const bytes = await readFile(artifact)
  const artifactSha256 = createHash('sha256').update(bytes).digest('hex')
  if (expectedArtifactSha256 !== undefined && artifactSha256 !== expectedArtifactSha256) {
    fail('b0-authorization', 'artifact sha256 drift')
  }

  const parsed = object(parseJson(bytes), 'b0-authorization', 'B0 artifact')
  const storedChecks = array(parsed.required_checks, 'b0-authorization', 'required_checks')
  if (storedChecks.length !== 2) {
    fail('b0-authorization', 'artifact required-check count drift')
  }
  const phases: B0AntiFixturePhase[] = ['pre-calibration', 'pre-holdout']
  for (const [index, phase] of phases.entries()) {
    const attestationPath = requiredCheckAttestations[index]
    if (
      !path.isAbsolute(attestationPath) ||
      (await realpath(attestationPath)) !== attestationPath
    ) {
      fail('b0-authorization', 'required-check attestation path must be absolute and canonical')
    }
    const expectedRelpath = `source-gate-selection/checks/${phase}.json`
    if (path.relative(path.dirname(artifact), attestationPath) !== expectedRelpath) {
      fail('b0-authorization', 'required-check attestation path drift')
    }
    const expectedAttestation = await buildB0SourceGateAntiFixtureAttestation(
      root,
      phase,
      expectedB0Sha,
      visualManifestSha256,
      fixtureManifestSha256,
    )
    const attestationBytes = await readFile(attestationPath)
    if (attestationBytes.toString('utf8') !== `${canonicalJson(expectedAttestation)}\n`) {
      fail('b0-authorization', 'required-check attestation would not be reproduced')
    }
    const attestationSha256 = createHash('sha256').update(attestationBytes).digest('hex')
    const expectedCheck = {
      phase,
      command: 'bun scripts/check-hanonly-production-policy.ts --b0-source-gate-anti-fixture',
      checker_endpoint_sha256: expectedAttestation.checker_endpoint_sha256,
      manifest_sha256: visualManifestSha256,
      source_gate_fixture_manifest_sha256: fixtureManifestSha256,
      attestation_relpath: expectedRelpath,
      attestation_sha256: attestationSha256,
      b0_sha: expectedB0Sha,
      result: 'pass',
    }
    if (
      !deepEqual(
        object(storedChecks[index], 'b0-authorization', 'required-check entry'),
        expectedCheck,
      )
    ) {
      fail('b0-authorization', 'artifact required-check entry drift')
    }
  }
  const selected = parsed.selected_candidate_id
  if (!['S25L4', 'S25L5', 'S25L6', 'S25L7'].includes(String(selected))) {
    fail('b0-authorization', 'selected candidate is invalid')
  }
  if (flag(args, '--verify-selected-ratio-in-production')) {
    const candidates = array(parsed.candidates, 'b0-authorization', 'candidates')
    const selectedCandidate = candidates.find(
      (candidate) =>
        object(candidate, 'b0-authorization', 'candidate').id === parsed.selected_candidate_id,
    )
    if (!selectedCandidate) fail('b0-authorization', 'selected candidate is missing')
    createHash('sha256').update(canonicalJson(selectedCandidate)).digest('hex')
  }

  const result = Bun.spawnSync({
    cmd: [
      'python3',
      'scripts/hanonly_evidence_ledger.py',
      'validate-b0-artifact',
      '--repo-root',
      await realpath(root),
      '--artifact',
      artifact,
      '--b0-sha',
      expectedB0Sha,
      '--visual-manifest-sha256',
      visualManifestSha256,
      '--source-gate-fixture-manifest-sha256',
      fixtureManifestSha256,
    ],
    cwd: root,
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1' },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  if (result.exitCode !== 0) {
    fail('b0-authorization', 'ledger B0 artifact validator failed')
  }
  if (result.stdout.toString() !== 'PASS B0 frozen artifact\n') {
    fail('b0-authorization', 'ledger B0 artifact validator output drift')
  }
  return flag(args, '--emit-artifact-sha256') ? artifactSha256 : undefined
}

type CargoCompilerArtifact = {
  reason?: unknown
  package_id?: unknown
  target?: {
    kind?: unknown
    name?: unknown
  }
  profile?: {
    test?: unknown
  }
  features?: unknown
  executable?: unknown
}

function r51Args(args: readonly string[], endpoint: string): string[] {
  const filtered = args.filter((value) => value !== endpoint)
  if (filtered.length !== args.length - 1) {
    fail('r51-argv', `${endpoint} must appear exactly once`)
  }
  return filtered
}

function hasR51Option(args: readonly string[], option: string): boolean {
  return args.some((value) => value === option || value.startsWith(`${option}=`))
}

function r51Python(
  root: string,
  command: string,
  args: readonly string[],
  category: string,
): string {
  const result = Bun.spawnSync({
    cmd: ['python3', 'scripts/hanonly_evidence_ledger.py', command, '--repo-root', root, ...args],
    cwd: root,
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1' },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  if (result.exitCode !== 0) {
    fail(category, 'R51 evidence ledger validation failed')
  }
  return result.stdout.toString()
}

export function r51EvidenceExecutable(cargoJson: string): string {
  const artifacts: CargoCompilerArtifact[] = []
  for (const line of cargoJson.split('\n')) {
    if (!line) continue
    if (
      /^Storage: \d+(?:\.\d+)? GiB free, target \d+(?:\.\d+)? GiB, ui\/\.next \d+(?:\.\d+)? GiB$/.test(
        line,
      )
    ) {
      continue
    }
    let message: CargoCompilerArtifact
    try {
      message = JSON.parse(line) as CargoCompilerArtifact
    } catch {
      fail('r51-b0-preflight', 'Cargo returned invalid JSON messages')
    }
    if (
      message.reason === 'compiler-artifact' &&
      message.profile?.test === true &&
      Array.isArray(message.target?.kind) &&
      deepEqual(message.target.kind, ['lib']) &&
      message.target.name === 'koharu_app' &&
      typeof message.package_id === 'string' &&
      /(?:^|[/#])koharu-app(?:[#@]|$)/.test(message.package_id) &&
      typeof message.executable === 'string'
    ) {
      if (!deepEqual(message.features, ['hanonly-test-evidence'])) {
        fail('r51-b0-preflight', 'evidence lib-test Cargo feature set drift')
      }
      artifacts.push(message)
    }
  }
  if (artifacts.length !== 1) {
    fail('r51-b0-preflight', 'expected exactly one koharu-app evidence lib-test executable')
  }
  return artifacts[0].executable as string
}

function runR51Gate(
  root: string,
  command: readonly string[],
  env?: Record<string, string>,
): Buffer {
  const result = Bun.spawnSync({
    cmd: [...command],
    cwd: root,
    env: { ...process.env, ...env },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const output = Buffer.concat([result.stdout, result.stderr])
  if (result.exitCode !== 0) {
    fail('r51-b0-preflight', `gate failed: ${command.join(' ')}`)
  }
  return output
}

export const r51MarkerInventoryCommand = [
  'bun',
  'scripts/check-hanonly-production-policy.ts',
  '--validate-red-test-state',
  'b0',
] as const

function r51CustodySnapshot(root: string, args: readonly string[]): string {
  const forwarded = [
    '--r51-contract',
    requiredArg(args, '--r51-contract'),
    '--operative-plan',
    requiredArg(args, '--operative-plan'),
    '--r51-test-spec',
    requiredArg(args, '--r51-test-spec'),
    '--base-production-contract',
    requiredArg(args, '--base-production-contract'),
    '--freeze-receipt',
    requiredArg(args, '--freeze-receipt'),
    '--historical-inventory',
    requiredArg(args, '--historical-inventory'),
    '--ciphertext',
    requiredArg(args, '--ciphertext'),
  ]
  return r51Python(root, 'snapshot-r51-preflight-custody', forwarded, 'r51-b0-preflight-custody')
}

async function writeR51EvidenceFile(filePath: string, bytes: Buffer): Promise<void> {
  const parent = path.dirname(filePath)
  const parentStat = await stat(parent)
  if (
    (await realpath(parent)) !== parent ||
    !parentStat.isDirectory() ||
    parentStat.uid !== process.getuid?.() ||
    (parentStat.mode & 0o7777) !== 0o700
  ) {
    fail('r51-b0-preflight', 'preflight output parent must be same-owner mode-0700')
  }
  let handle: FileHandle
  try {
    handle = await open(
      filePath,
      constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
      0o600,
    )
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error
    const existingHandle = await open(filePath, constants.O_RDONLY | constants.O_NOFOLLOW)
    try {
      const existing = await existingHandle.stat()
      if (
        !existing.isFile() ||
        existing.uid !== process.getuid?.() ||
        (existing.mode & 0o7777) !== 0o600 ||
        !(await existingHandle.readFile()).equals(bytes)
      ) {
        fail('r51-b0-preflight', 'existing preflight evidence drift')
      }
    } finally {
      await existingHandle.close()
    }
    return
  }
  try {
    await handle.writeFile(bytes)
    await handle.sync()
  } finally {
    await handle.close()
  }
}

async function runR51PreflightGates(
  root: string,
  outputPath: string,
): Promise<{ gateResultsPath: string; stagedRedPath: string }> {
  const gateResults = {
    directed_source_gate_regressions: 'pass',
    directed_ppocr_regressions: 'pass',
    b0_owned_tests: 'pass',
    default_workspace_tests: 'pass',
    workspace_all_targets_check: 'pass',
    generated: 'pass',
    format: 'pass',
    policy: 'pass',
    anti_fixture: 'pass',
    r51_marker_inventory: 'pass',
    staged_red_t2: 'pass',
    staged_red_t3: 'pass',
  } as const
  const commands: Record<keyof typeof gateResults, readonly string[]> = {
    directed_source_gate_regressions: [
      'bun',
      'cargo',
      'test',
      '-p',
      'koharu-app',
      '--features',
      'hanonly-test-evidence',
      'pipeline::engines::source_language_gate::tests',
      '--',
      '--nocapture',
    ],
    directed_ppocr_regressions: [
      'bun',
      'cargo',
      'test',
      '-p',
      'koharu-ml',
      'pp_ocr_v5',
      '--',
      '--nocapture',
    ],
    b0_owned_tests: [
      'bun',
      'cargo',
      'test',
      '-p',
      'koharu-app',
      'hanonly_pre_b1_red_t2_source_gate_ratio_contract',
      '--',
      '--nocapture',
    ],
    default_workspace_tests: ['bun', 'cargo', 'test', '--workspace', '--tests'],
    workspace_all_targets_check: ['bun', 'cargo', 'check', '--workspace', '--all-targets'],
    generated: ['bun', 'run', 'check:generated'],
    format: ['bun', 'run', 'format:check'],
    policy: ['bun', 'test', 'scripts/check-hanonly-production-policy.test.ts'],
    anti_fixture: [
      'bun',
      'scripts/check-hanonly-production-policy.ts',
      '--scan-b0-source-gate-anti-fixture',
    ],
    r51_marker_inventory: r51MarkerInventoryCommand,
    staged_red_t2: ['true'],
    staged_red_t3: ['true'],
  }
  for (const [name, command] of Object.entries(commands)) {
    if (name.startsWith('staged_red_')) continue
    runR51Gate(root, command)
    if (name === 'directed_ppocr_regressions') {
      runR51Gate(root, [
        'bun',
        'cargo',
        'test',
        '-p',
        'koharu-ml',
        'hanonly_pre_b1_red_t2_crop_local_ppocr_contract',
        '--',
        '--nocapture',
      ])
    }
    if (name === 'policy') {
      runR51Gate(root, ['python3', '-m', 'unittest', 'scripts/hanonly_evidence_ledger_test.py'], {
        PYTHONDONTWRITEBYTECODE: '1',
      })
    }
  }

  runR51Gate(root, ['bun', 'run', 'clean:rust:dev'])
  const list = runR51Gate(root, [
    'bun',
    'cargo',
    'test',
    '--workspace',
    '--tests',
    '--',
    '--list',
    '--ignored',
  ])
  const listed = list
    .toString('utf8')
    .split('\n')
    .filter((line) => line.endsWith(': test'))
    .map((line) => line.slice(0, -': test'.length))
  const stagedHashes: Record<string, string> = {}
  const stagedRedDir = path.join(path.dirname(outputPath), 'r51-staged-red')
  try {
    await mkdir(stagedRedDir, { mode: 0o700 })
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error
  }
  const stagedRedDirStat = await stat(stagedRedDir)
  if (
    (await realpath(stagedRedDir)) !== stagedRedDir ||
    !stagedRedDirStat.isDirectory() ||
    stagedRedDirStat.uid !== process.getuid?.() ||
    (stagedRedDirStat.mode & 0o7777) !== 0o700
  ) {
    fail('r51-b0-preflight', 'staged RED evidence directory is insecure')
  }
  for (const id of [...expectedB0B1MarkerIds, ...expectedGreenCRedIds]) {
    const matches = listed.filter((name) => name.split('::').at(-1) === id)
    if (matches.length !== 1) {
      fail('r51-b0-preflight', `staged RED identity drift: ${id}`)
    }
    const result = Bun.spawnSync({
      cmd: [
        'bun',
        'cargo',
        'test',
        '--workspace',
        '--tests',
        matches[0],
        '--',
        '--exact',
        '--ignored',
        '--nocapture',
      ],
      cwd: root,
      stdout: 'pipe',
      stderr: 'pipe',
    })
    const bytes = Buffer.concat([result.stdout, result.stderr])
    const text = bytes.toString('utf8')
    if (
      result.exitCode === 0 ||
      !text.includes('running 1 test') ||
      !text.includes('FAILED') ||
      !text.includes('test result: FAILED')
    ) {
      fail('r51-b0-preflight', `staged RED did not fail exactly: ${id}`)
    }
    await writeR51EvidenceFile(path.join(stagedRedDir, `${id}.log`), bytes)
    stagedHashes[id] = createHash('sha256').update(bytes).digest('hex')
  }
  runR51Gate(root, ['git', 'diff', '--exit-code'])
  runR51Gate(root, ['git', 'diff', '--cached', '--exit-code'])
  const status = runR51Gate(root, ['git', 'status', '--porcelain=v1', '--untracked-files=all'])
  if (status.length !== 0) fail('r51-b0-preflight', 'gate execution dirtied the B0 worktree')

  const parent = path.dirname(outputPath)
  const gateResultsPath = path.join(parent, 'r51-preflight-gates.json')
  const stagedRedPath = path.join(parent, 'r51-staged-red-hashes.json')
  await writeR51EvidenceFile(gateResultsPath, Buffer.from(canonicalJson(gateResults)))
  await writeR51EvidenceFile(stagedRedPath, Buffer.from(canonicalJson(stagedHashes)))
  return { gateResultsPath, stagedRedPath }
}

export async function writeR51B0PreflightAttestation(
  root: string,
  args: readonly string[],
): Promise<string> {
  const forwarded = r51Args(args, '--write-r51-b0-preflight-attestation')
  if (
    hasR51Option(forwarded, '--repo-root') ||
    hasR51Option(forwarded, '--evidence-test-executable') ||
    hasR51Option(forwarded, '--cargo-target-dir') ||
    hasR51Option(forwarded, '--gate-results') ||
    hasR51Option(forwarded, '--staged-red-log')
  ) {
    fail('r51-argv', 'R51 preflight reserves internal evidence arguments')
  }
  const outputPath = requiredArg(args, '--output')
  const canonicalRoot = await realpath(root)
  const custodySnapshot = r51CustodySnapshot(canonicalRoot, args)
  const cargoTargetDir = process.env.CARGO_TARGET_DIR
  if (
    !cargoTargetDir ||
    !path.isAbsolute(cargoTargetDir) ||
    (await realpath(cargoTargetDir)) !== cargoTargetDir
  ) {
    fail('r51-b0-preflight', 'canonical CARGO_TARGET_DIR is required')
  }
  let existingBytes: Buffer | undefined
  try {
    const existingHandle = await open(outputPath, constants.O_RDONLY | constants.O_NOFOLLOW)
    try {
      existingBytes = await existingHandle.readFile()
    } finally {
      await existingHandle.close()
    }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
  }
  if (existingBytes) {
    let existing: JsonObject
    try {
      existing = JSON.parse(existingBytes.toString('utf8')) as JsonObject
    } catch {
      fail('r51-b0-preflight', 'existing preflight attestation is invalid')
    }
    if (typeof existing.evidence_test_executable_path !== 'string') {
      fail('r51-b0-preflight', 'existing preflight executable binding is invalid')
    }
    const parent = path.dirname(outputPath)
    if (r51CustodySnapshot(canonicalRoot, args) !== custodySnapshot) {
      fail('r51-b0-preflight-custody', 'custody changed during preflight rerun')
    }
    return r51Python(
      canonicalRoot,
      'write-r51-b0-preflight-attestation',
      [
        ...forwarded,
        '--gate-results',
        path.join(parent, 'r51-preflight-gates.json'),
        '--staged-red-log',
        path.join(parent, 'r51-staged-red-hashes.json'),
        '--evidence-test-executable',
        existing.evidence_test_executable_path,
        '--cargo-target-dir',
        cargoTargetDir,
      ],
      'r51-b0-preflight',
    )
  }
  const harness = await readRepoText(
    root,
    'crates/koharu-app/src/pipeline/d0_visual_manifest_harness.rs',
    'R51 evidence harness',
  )
  if (
    !harness.includes('#[cfg(all(test, feature = "hanonly-test-evidence"))]') ||
    !harness.includes('fn han_only_source_gate_crop_selection_matrix()')
  ) {
    fail('r51-b0-preflight', 'required R51 evidence harness is unavailable')
  }
  const generated = await runR51PreflightGates(root, outputPath)
  const cargo = Bun.spawnSync({
    cmd: [
      'bun',
      'cargo',
      'test',
      '-p',
      'koharu-app',
      '--features',
      'hanonly-test-evidence',
      '--no-run',
      '--message-format=json',
    ],
    cwd: root,
    stdout: 'pipe',
    stderr: 'pipe',
  })
  if (cargo.exitCode !== 0) {
    fail('r51-b0-preflight', 'exact R51 evidence test build failed')
  }
  const executable = r51EvidenceExecutable(cargo.stdout.toString())
  if (r51CustodySnapshot(canonicalRoot, args) !== custodySnapshot) {
    fail('r51-b0-preflight-custody', 'custody changed during preflight tests')
  }
  return r51Python(
    canonicalRoot,
    'write-r51-b0-preflight-attestation',
    [
      ...forwarded,
      '--gate-results',
      generated.gateResultsPath,
      '--staged-red-log',
      generated.stagedRedPath,
      '--evidence-test-executable',
      executable,
      '--cargo-target-dir',
      cargoTargetDir,
    ],
    'r51-b0-preflight',
  )
}

export async function validateR51B0Authorization(
  root: string,
  args: readonly string[],
): Promise<string> {
  const forwarded = r51Args(args, '--validate-r51-b0-authorization')
  if (hasR51Option(forwarded, '--repo-root')) {
    fail('r51-argv', 'R51 authorization reserves --repo-root')
  }
  return r51Python(
    await realpath(root),
    'validate-r51-b0-authorization',
    forwarded,
    'r51-b0-authorization',
  )
}

async function main(): Promise<void> {
  const args = process.argv.slice(2)
  if (deepEqual(args, ['--test-dependency-inventory'])) {
    const snapshotDir = process.env.HANONLY_ORIGINAL_SNAPSHOT_DIR
    if (!snapshotDir) fail('snapshot-env', 'HANONLY_ORIGINAL_SNAPSHOT_DIR is required')
    await runDependencyInventory(repoRoot, snapshotDir)
    process.stdout.write('PASS: hanonly production dependency inventory policy\n')
    return
  }
  if (args.includes('--validate-red-test-state')) {
    const state = requiredArg(args, '--validate-red-test-state')
    if (state !== 'b0' && state !== 'final') {
      fail('red-test-state', '--validate-red-test-state must be b0 or final')
    }
    await runRedTestState(repoRoot, state)
    process.stdout.write(`PASS: hanonly ${state} red test state\n`)
    return
  }
  if (deepEqual(args, ['--release-feature-inventory'])) {
    await runReleaseFeatureInventory(repoRoot)
    process.stdout.write('PASS: hanonly release feature inventory\n')
    return
  }
  if (args.includes('--verify-frozen-interpreter')) {
    runFrozenInterpreterCheck(
      repoRoot,
      requiredArg(args, '--b0-sha'),
      requiredArg(args, '--impl-sha'),
    )
    process.stdout.write('PASS: hanonly frozen interpreter\n')
    return
  }
  if (args.includes('--verify-generated-rust')) {
    await runGeneratedRustAudit(
      repoRoot,
      requiredArg(args, '--cargo-default-messages'),
      requiredArg(args, '--cargo-evidence-messages'),
    )
    process.stdout.write('PASS: hanonly generated Rust audit\n')
    return
  }
  if (args.includes('--validate-b0-authorization')) {
    const digest = await validateB0Authorization(repoRoot, args)
    if (digest) process.stdout.write(`${digest}\n`)
    return
  }
  if (args.includes('--write-r51-b0-preflight-attestation')) {
    process.stdout.write(await writeR51B0PreflightAttestation(repoRoot, args))
    return
  }
  if (args.includes('--validate-r51-b0-authorization')) {
    process.stdout.write(await validateR51B0Authorization(repoRoot, args))
    return
  }
  if (deepEqual(args, ['--b0-source-gate-anti-fixture'])) {
    await runB0SourceGateAntiFixture(repoRoot)
    process.stdout.write('PASS: hanonly b0 source gate anti-fixture\n')
    return
  }
  if (deepEqual(args, ['--scan-b0-source-gate-anti-fixture'])) {
    await scanB0SourceGateAntiFixture(repoRoot)
    process.stdout.write('PASS: hanonly b0 source gate anti-fixture scan\n')
    return
  }
  fail('argv', 'expected a known HanOnly production policy mode')
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
