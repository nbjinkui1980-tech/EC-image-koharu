import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('..', import.meta.url))
const requiredFields = [
  'package',
  'advisory',
  'severity',
  'dependencyPath',
  'reachability',
  'owner',
  'reason',
  'expiresOn',
] as const
const reachabilityValues = new Set(['runtime', 'development', 'build', 'test', 'unreachable'])

export type AllowlistEntry = Record<(typeof requiredFields)[number], string>

type AuditAdvisory = {
  url?: unknown
  severity?: unknown
}

type Finding = {
  package: string
  advisory: string
  severity: string
}

export function validateAllowlist(value: unknown, today = new Date()): AllowlistEntry[] {
  if (!Array.isArray(value)) throw new Error('allowlist must be an array')

  const entries = value.map((entry, index) => {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) {
      throw new Error(`allowlist entry ${index} must be an object`)
    }

    const candidate = entry as Record<string, unknown>
    for (const field of requiredFields) {
      if (typeof candidate[field] !== 'string' || !candidate[field].trim()) {
        throw new Error(`allowlist entry ${index} ${field} is required`)
      }
    }
    if (!reachabilityValues.has(candidate.reachability as string)) {
      throw new Error(`allowlist entry ${index} reachability is invalid`)
    }

    const expiresOn = candidate.expiresOn as string
    const expiry = Date.parse(`${expiresOn}T00:00:00.000Z`)
    if (!/^\d{4}-\d{2}-\d{2}$/.test(expiresOn) || Number.isNaN(expiry)) {
      throw new Error(`allowlist entry ${index} expiresOn is invalid`)
    }
    if (expiry <= Date.UTC(today.getUTCFullYear(), today.getUTCMonth(), today.getUTCDate())) {
      throw new Error(`allowlist entry ${index} is expired`)
    }

    return {
      ...(candidate as AllowlistEntry),
      advisory: (candidate.advisory as string).toUpperCase(),
      severity: (candidate.severity as string).toLowerCase(),
    }
  })

  const keys = new Set<string>()
  for (const entry of entries) {
    const key = `${entry.package}\u0000${entry.advisory}`
    if (keys.has(key))
      throw new Error(`allowlist entry ${entry.package} ${entry.advisory} is duplicated`)
    keys.add(key)
  }

  return entries
}

function advisoryId(url: unknown): string | undefined {
  if (typeof url !== 'string') return undefined
  return url
    .match(/(?:GHSA-[a-z0-9]{4}-[a-z0-9]{4}-[a-z0-9]{4}|CVE-\d{4}-\d+)/i)?.[0]
    ?.toUpperCase()
}

function findingsFromAudit(value: unknown): Finding[] {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('audit output must be a package-to-advisories object')
  }

  const findings: Finding[] = []
  for (const [packageName, advisories] of Object.entries(value as Record<string, unknown>)) {
    if (!Array.isArray(advisories))
      throw new Error(`audit advisories for ${packageName} must be an array`)
    for (const advisory of advisories as AuditAdvisory[]) {
      const id = advisoryId(advisory.url)
      if (!id || typeof advisory.severity !== 'string') {
        throw new Error(`audit advisory for ${packageName} is missing a stable ID or severity`)
      }
      findings.push({
        package: packageName,
        advisory: id,
        severity: advisory.severity.toLowerCase(),
      })
    }
  }
  return findings
}

export function evaluateAudit(audit: unknown, allowlist: unknown, today = new Date()) {
  let entries: AllowlistEntry[]
  try {
    entries = validateAllowlist(allowlist, today)
  } catch (error) {
    return { ok: false, findings: [], errors: [(error as Error).message] }
  }

  let findings: Finding[]
  try {
    findings = findingsFromAudit(audit)
  } catch (error) {
    return { ok: false, findings: [], errors: [(error as Error).message] }
  }

  const permitted = new Map(
    entries.map((entry) => [`${entry.package}\u0000${entry.advisory}`, entry]),
  )
  const errors = findings.flatMap((finding) => {
    const entry = permitted.get(`${finding.package}\u0000${finding.advisory}`)
    if (!entry) {
      return [
        `unapproved ${finding.package} ${finding.advisory} severity=${finding.severity} dependencyPath=unclassified owner=unassigned expiresOn=n/a`,
      ]
    }
    if (entry.severity !== finding.severity) {
      return [
        `severity drift ${finding.package} ${finding.advisory}: audit=${finding.severity} allowlist=${entry.severity}`,
      ]
    }
    if (entry.reachability === 'runtime' && ['high', 'critical'].includes(finding.severity)) {
      return [
        `runtime ${finding.severity} may not be allowlisted: ${finding.package} ${finding.advisory}`,
      ]
    }
    return []
  })

  return { ok: errors.length === 0, findings, errors }
}

function parseAudit(stdout: string): unknown {
  const start = stdout.indexOf('{')
  if (start < 0) throw new Error('bun audit did not return JSON')
  return JSON.parse(stdout.slice(start))
}

if (import.meta.main) {
  const command = Bun.spawnSync({
    cmd: ['bun', 'audit', '--json', '--registry', 'https://registry.npmjs.org'],
    cwd: root,
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const allowlist = await Bun.file(`${root}/scripts/supply-chain-allowlist.json`).json()

  try {
    const result = evaluateAudit(parseAudit(command.stdout.toString()), allowlist)
    if (!result.ok) {
      process.stderr.write(`${result.errors.join('\n')}\n`)
      process.exit(1)
    }
    process.stdout.write('PASS: supply-chain audit policy\n')
  } catch (error) {
    process.stderr.write(`supply-chain policy failed: ${(error as Error).message}\n`)
    process.exit(1)
  }
}
