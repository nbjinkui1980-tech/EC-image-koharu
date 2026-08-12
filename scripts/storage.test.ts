import { describe, expect, test } from 'bun:test'
import { spawnSync } from 'node:child_process'
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import path from 'node:path'

import {
  cargoSweepFailed,
  cargoTargetViolation,
  GIB,
  isSeparateMountedFilesystem,
  pruneTargetViolation,
  sharedTargetViolation,
  shouldPruneTarget,
  storageViolations,
} from './storage'

const localCargoTest = process.platform === 'darwin' && !process.env.CI ? test : test.skip
const externalFixtureRoot = '/Volumes/G/EC-image-koharu/tmp/storage-tests'

function makeExternalFixture(prefix: string): string {
  const volumeDevice = statSync('/Volumes/G').dev
  if (
    !isSeparateMountedFilesystem(
      volumeDevice,
      statSync('/').dev,
      statSync('/System/Volumes/Data').dev,
    )
  ) {
    throw new Error('/Volumes/G is not a separate mounted filesystem')
  }
  mkdirSync(externalFixtureRoot, { recursive: true })
  return mkdtempSync(path.join(externalFixtureRoot, prefix))
}

describe('isSeparateMountedFilesystem', () => {
  test('rejects an unmounted volume directory on either macOS system filesystem', () => {
    expect(isSeparateMountedFilesystem(30, 10, 20)).toBe(true)
    expect(isSeparateMountedFilesystem(10, 10, 20)).toBe(false)
    expect(isSeparateMountedFilesystem(20, 10, 20)).toBe(false)
  })
})

describe('storageViolations', () => {
  test('enforces the free-space and generated-cache ceilings', () => {
    expect(
      storageViolations({
        systemFreeBytes: 19 * GIB,
        externalFreeBytes: 18 * GIB,
        targetBytes: 101 * GIB,
        nextBytes: 2 * GIB,
      }),
    ).toEqual([
      'System Data free space is below 20 GiB.',
      'G volume free space is below 20 GiB.',
      'target exceeds 100 GiB.',
      'ui/.next exceeds 1 GiB.',
    ])
  })

  test('accepts a workspace below every ceiling', () => {
    expect(
      storageViolations({
        systemFreeBytes: 20 * GIB,
        externalFreeBytes: 20 * GIB,
        targetBytes: 100 * GIB,
        nextBytes: 1 * GIB,
      }),
    ).toEqual([])
  })
})

describe('direct Cargo entrypoint', () => {
  localCargoTest('guards the default absolute Cargo executable only in Koharu', () => {
    const root = path.resolve(import.meta.dir, '..')
    const cargo = '/Users/jinkui/.cargo/bin/cargo'
    const hostileTarget = '/private/tmp/koharu-absolute-cargo-override'
    const guarded = spawnSync(cargo, ['metadata', '--no-deps', '--format-version', '1'], {
      cwd: root,
      encoding: 'utf8',
      env: { ...process.env, CARGO_TARGET_DIR: hostileTarget },
    })

    expect(guarded.status, guarded.stderr).toBe(0)
    const metadata = JSON.parse(
      guarded.stdout
        .trim()
        .split('\n')
        .findLast((line) => line.startsWith('{'))!,
    ) as { target_directory: string }
    expect(metadata.target_directory).toBe('/Volumes/G/EC-image-koharu/target')

    const outside = spawnSync(cargo, ['--version'], {
      cwd: '/private/tmp',
      encoding: 'utf8',
      env: process.env,
    })
    expect(outside.status, outside.stderr).toBe(0)
    expect(outside.stdout).toStartWith('cargo ')
  })

  localCargoTest('keeps bun cargo on the guarded entrypoint', () => {
    const root = path.resolve(import.meta.dir, '..')
    const fakeBin = makeExternalFixture('cargo-bun-probe-')
    writeFileSync(path.join(fakeBin, 'cargo'), '#!/bin/sh\nexit 0\n')
    chmodSync(path.join(fakeBin, 'cargo'), 0o755)
    try {
      const result = spawnSync(
        'bun',
        ['cargo', 'check', '--target-dir', '/private/tmp/koharu-bun-cargo-override'],
        {
          cwd: root,
          encoding: 'utf8',
          env: { ...process.env, PATH: `${fakeBin}:${process.env.PATH}` },
        },
      )
      expect(result.status).not.toBe(0)
      expect(result.stderr).toContain('Cargo target/build directory overrides are forbidden')
    } finally {
      rmSync(fakeBin, { recursive: true, force: true })
    }
  })

  localCargoTest(
    'overrides a hostile system-temporary Cargo target with the shared G target',
    () => {
      const root = path.resolve(import.meta.dir, '..')
      const result = spawnSync(
        path.join(import.meta.dir, 'cargo-command.sh'),
        ['metadata', '--no-deps', '--format-version', '1'],
        {
          cwd: root,
          encoding: 'utf8',
          env: {
            ...process.env,
            CARGO_TARGET_DIR: '/private/tmp/koharu-guard-red-test',
            KOHARU_SHARED_TARGET_DIR: '/private/tmp/koharu-guard-red-test',
            KOHARU_TMPDIR: '/private/tmp/koharu-guard-red-test',
          },
        },
      )

      expect(result.status, `${result.error ?? ''}\n${result.stderr}`).toBe(0)
      const metadata = JSON.parse(
        result.stdout
          .trim()
          .split('\n')
          .findLast((line) => line.startsWith('{'))!,
      ) as { target_directory: string }
      expect(metadata.target_directory).toBe('/Volumes/G/EC-image-koharu/target')
    },
  )

  localCargoTest('rejects direct target flags and overrides hostile Cargo config', () => {
    const root = path.resolve(import.meta.dir, '..')
    const fixture = makeExternalFixture('cargo-guard-fixture-')
    const target = mkdtempSync('/private/tmp/koharu-cli-override-')
    const config = path.join(fixture, 'target-override.toml')
    mkdirSync(path.join(fixture, 'src'))
    writeFileSync(
      path.join(fixture, 'Cargo.toml'),
      '[package]\nname = "cargo-guard-fixture"\nversion = "0.0.0"\nedition = "2024"\n',
    )
    writeFileSync(path.join(fixture, 'src', 'lib.rs'), 'pub fn probe() {}\n')
    writeFileSync(config, `[build]\ntarget-dir = ${JSON.stringify(target)}\n`)

    try {
      const directTarget = spawnSync(
        path.join(import.meta.dir, 'cargo-command.sh'),
        ['check', '--manifest-path', path.join(fixture, 'Cargo.toml'), '--target-dir', target],
        { cwd: root, encoding: 'utf8', env: process.env },
      )
      expect(directTarget.status).not.toBe(0)
      expect(directTarget.stderr).toContain('Cargo target/build directory overrides are forbidden')

      for (const args of [
        [
          '--config',
          `build.target-dir="${target}"`,
          'metadata',
          '--no-deps',
          '--format-version',
          '1',
        ],
        [
          '--config',
          `"build"."target-dir"="${target}"`,
          'metadata',
          '--no-deps',
          '--format-version',
          '1',
        ],
        ['--config', config, 'metadata', '--no-deps', '--format-version', '1'],
        [`--config=${config}`, 'metadata', '--no-deps', '--format-version', '1'],
      ]) {
        const result = spawnSync(path.join(import.meta.dir, 'cargo-command.sh'), args, {
          cwd: root,
          encoding: 'utf8',
          env: process.env,
        })

        expect(result.status, result.stderr).toBe(0)
        const metadata = JSON.parse(
          result.stdout
            .trim()
            .split('\n')
            .findLast((line) => line.startsWith('{'))!,
        ) as { target_directory: string; build_directory: string }
        expect(metadata.target_directory).toBe('/Volumes/G/EC-image-koharu/target')
        expect(metadata.build_directory).toBe('/Volumes/G/EC-image-koharu/target')
      }
    } finally {
      rmSync(fixture, { recursive: true, force: true })
      rmSync(target, { recursive: true, force: true })
    }
  })

  localCargoTest('allows safe Cargo config and application arguments after --', () => {
    const root = path.resolve(import.meta.dir, '..')
    const wrapper = path.join(import.meta.dir, 'cargo-command.sh')
    const safeConfig = spawnSync(
      wrapper,
      ['metadata', '--no-deps', '--format-version', '1', '--config', 'net.offline=true'],
      { cwd: root, encoding: 'utf8', env: process.env },
    )
    expect(safeConfig.status, safeConfig.stderr).toBe(0)

    const externalSubcommand = spawnSync(wrapper, ['fmt', '--version'], {
      cwd: root,
      encoding: 'utf8',
      env: process.env,
    })
    expect(externalSubcommand.status, externalSubcommand.stderr).toBe(0)

    const probeBin = makeExternalFixture('cargo-config-probe-')
    const probeMarker = path.join(probeBin, 'arguments.txt')
    writeFileSync(
      path.join(probeBin, 'cargo-config-probe'),
      '#!/bin/sh\nprintf \'%s\\n\' "$@" > "$CARGO_CONFIG_PROBE_MARKER"\n',
    )
    chmodSync(path.join(probeBin, 'cargo-config-probe'), 0o755)
    try {
      const externalConfig = spawnSync(wrapper, ['config-probe', '--config', 'tool-config.toml'], {
        cwd: root,
        encoding: 'utf8',
        env: {
          ...process.env,
          CARGO_CONFIG_PROBE_MARKER: probeMarker,
          PATH: `${probeBin}:${process.env.PATH}`,
        },
      })
      expect(externalConfig.status, externalConfig.stderr).toBe(0)
      expect(readFileSync(probeMarker, 'utf8')).toContain('--config\ntool-config.toml')

      const externalTarget = spawnSync(
        wrapper,
        ['config-probe', '--target-dir', '/private/tmp/koharu-external-target'],
        {
          cwd: root,
          encoding: 'utf8',
          env: {
            ...process.env,
            CARGO_CONFIG_PROBE_MARKER: probeMarker,
            PATH: `${probeBin}:${process.env.PATH}`,
          },
        },
      )
      expect(externalTarget.status).not.toBe(0)
      expect(externalTarget.stderr).toContain(
        'Cargo target/build directory overrides are forbidden',
      )
    } finally {
      rmSync(probeBin, { recursive: true, force: true })
    }

    const fakeBin = makeExternalFixture('cargo-guard-bin-')
    writeFileSync(path.join(fakeBin, 'bun'), '#!/bin/sh\nexit 0\n')
    chmodSync(path.join(fakeBin, 'bun'), 0o755)
    try {
      const applicationConfig = spawnSync(wrapper, ['run', '--', '--config', 'app.toml'], {
        cwd: root,
        encoding: 'utf8',
        env: { ...process.env, PATH: `${fakeBin}:${process.env.PATH}` },
      })
      expect(applicationConfig.status, applicationConfig.stderr).toBe(0)
    } finally {
      rmSync(fakeBin, { recursive: true, force: true })
    }
  })

  localCargoTest(
    'routes zsh cargo and bun cargo in every Koharu worktree',
    () => {
      const root = path.resolve(import.meta.dir, '..')
      const worktrees = spawnSync('git', ['worktree', 'list', '--porcelain'], {
        cwd: root,
        encoding: 'utf8',
      })
        .stdout.split('\n')
        .filter((line) => line.startsWith('worktree '))
        .map((line) => line.slice('worktree '.length))
      const hostileTarget = mkdtempSync('/private/tmp/koharu-zsh-override-')
      const fakeBin = makeExternalFixture('cargo-zsh-probe-')
      writeFileSync(
        path.join(fakeBin, 'bun'),
        '#!/bin/sh\ncase "$1" in\n  */storage.ts) exit 0 ;;\n  */dev.ts) case " $* " in *" cargo -- --config "*) exit 7 ;; esac; printf \'%s\\n%s\\n%s\\n%s\\n%s\\n\' "$CARGO_TARGET_DIR" "$KOHARU_TMPDIR" "$TMPDIR" "$TMP" "$TEMP" ;;\n  *) printf \'passthrough:%s\\n\' "$*" ;;\nesac\n',
      )
      chmodSync(path.join(fakeBin, 'bun'), 0o755)

      try {
        expect(worktrees).toContain(root)
        for (const worktree of worktrees) {
          for (const invocation of [
            'cargo metadata --no-deps --format-version 1',
            'bun cargo metadata --no-deps --format-version 1',
            'bun run cargo metadata --no-deps --format-version 1',
            'bun cargo -- check',
            'bun run cargo -- check',
          ]) {
            const result = spawnSync(
              'zsh',
              [
                '-lc',
                `cd ${JSON.stringify(worktree)} && CARGO_TARGET_DIR=${JSON.stringify(hostileTarget)} KOHARU_TMPDIR=${JSON.stringify(hostileTarget)} ${invocation}`,
              ],
              { encoding: 'utf8', env: { ...process.env, PATH: `${fakeBin}:${process.env.PATH}` } },
            )
            expect(result.status, result.stderr).toBe(0)
            expect(result.stdout.trim().split('\n').slice(-5)).toEqual([
              '/Volumes/G/EC-image-koharu/target',
              ...Array(4).fill('/Volumes/G/EC-image-koharu/tmp'),
            ])
          }

          const ordinaryBun = spawnSync(
            'zsh',
            ['-lc', `cd ${JSON.stringify(worktree)} && bun test`],
            {
              encoding: 'utf8',
              env: { ...process.env, PATH: `${fakeBin}:${process.env.PATH}` },
            },
          )
          expect(ordinaryBun.status, ordinaryBun.stderr).toBe(0)
          expect(ordinaryBun.stdout).toContain('passthrough:test')
        }

        const outside = makeExternalFixture('cargo-outside-')
        const outsideEnvironment = spawnSync(
          'zsh',
          [
            '-lc',
            `cd ${JSON.stringify(outside)} && printf '%s\\n%s\\n' "\${CARGO_TARGET_DIR-}" "\${TMPDIR-}"`,
          ],
          { encoding: 'utf8', env: process.env },
        )
        expect(outsideEnvironment.status, outsideEnvironment.stderr).toBe(0)
        expect(outsideEnvironment.stdout).not.toContain('/Volumes/G/EC-image-koharu')

        for (const invocation of ['bun cargo', 'bun run cargo']) {
          const outsideBun = spawnSync(
            'zsh',
            ['-lc', `cd ${JSON.stringify(outside)} && ${invocation}`],
            {
              encoding: 'utf8',
              env: { ...process.env, PATH: `${fakeBin}:${process.env.PATH}` },
            },
          )
          expect(outsideBun.status, outsideBun.stderr).toBe(0)
          expect(outsideBun.stdout).toContain(`passthrough:${invocation.slice(4)}`)
        }
        rmSync(outside, { recursive: true, force: true })
      } finally {
        rmSync(hostileTarget, { recursive: true, force: true })
        rmSync(fakeBin, { recursive: true, force: true })
      }
    },
    20_000,
  )
})

describe('storage check process boundary', () => {
  test('does not let an arbitrary CI variable bypass target validation', () => {
    const result = spawnSync('bun', [path.join(import.meta.dir, 'storage.ts'), 'check'], {
      encoding: 'utf8',
      env: {
        ...process.env,
        CI: '1',
        GITHUB_ACTIONS: 'true',
        CARGO_TARGET_DIR: '/private/tmp/koharu-ci-bypass',
        KOHARU_SHARED_TARGET_DIR: '/private/tmp/koharu-ci-bypass',
      },
    })

    expect(result.status).not.toBe(0)
    expect(result.stderr).toContain('Storage guard blocked this command')
  })
})

describe('cargoTargetViolation', () => {
  test('rejects Cargo targets under macOS system temporary storage', () => {
    expect(cargoTargetViolation('/tmp/koharu-sdd-ar01-t01')).toBe(
      'CARGO_TARGET_DIR must not use macOS system temporary storage.',
    )
    expect(cargoTargetViolation('/private/tmp/koharu-review-ar01-t03')).toBe(
      'CARGO_TARGET_DIR must not use macOS system temporary storage.',
    )
    expect(cargoTargetViolation('/System/Volumes/Data/private/tmp/koharu-sdd-ar01-final')).toBe(
      'CARGO_TARGET_DIR must not use macOS system temporary storage.',
    )
  })

  test('rejects a task-specific target when a shared target is configured', () => {
    expect(
      cargoTargetViolation(
        '/Volumes/G/EC-image-koharu/target/ar01-t04',
        '/Volumes/G/EC-image-koharu/target',
      ),
    ).toBe('CARGO_TARGET_DIR must equal KOHARU_SHARED_TARGET_DIR.')
  })

  test('accepts the configured shared external Cargo target', () => {
    expect(
      cargoTargetViolation(
        '/Volumes/G/EC-image-koharu/target',
        '/Volumes/G/EC-image-koharu/target',
      ),
    ).toBeUndefined()
  })
})

describe('shouldPruneTarget', () => {
  test('allows pruning only after the shared target exceeds 100 GiB', () => {
    expect(shouldPruneTarget(100 * GIB)).toBe(false)
    expect(shouldPruneTarget(101 * GIB)).toBe(true)
  })
})

describe('pruneTargetViolation', () => {
  test('requires the configured shared target on the G volume', () => {
    expect(pruneTargetViolation('/Volumes/G/EC-image-koharu/target')).toBe(
      'prune-rust requires KOHARU_SHARED_TARGET_DIR to protect the shared Cargo cache.',
    )
    expect(
      pruneTargetViolation('/Volumes/G/other-target', '/Volumes/G/EC-image-koharu/target'),
    ).toBe('CARGO_TARGET_DIR must equal KOHARU_SHARED_TARGET_DIR.')
    expect(
      pruneTargetViolation(
        '/Volumes/F/EC-image-koharu/target',
        '/Volumes/F/EC-image-koharu/target',
      ),
    ).toBe('prune-rust only operates on the shared Cargo cache under /Volumes/G.')
    expect(
      pruneTargetViolation(
        '/Volumes/G/EC-image-koharu/target',
        '/Volumes/G/EC-image-koharu/target',
      ),
    ).toBeUndefined()
  })
})

describe('sharedTargetViolation', () => {
  test('requires the shared Cargo target to stay on the G volume', () => {
    expect(sharedTargetViolation()).toBe(
      'KOHARU_SHARED_TARGET_DIR is required; set it to /Volumes/G/EC-image-koharu/target.',
    )
    expect(sharedTargetViolation('/tmp/koharu-target')).toBe(
      'KOHARU_SHARED_TARGET_DIR must equal /Volumes/G/EC-image-koharu/target.',
    )
    expect(sharedTargetViolation('/Volumes/F/EC-image-koharu/target')).toBe(
      'KOHARU_SHARED_TARGET_DIR must equal /Volumes/G/EC-image-koharu/target.',
    )
    expect(sharedTargetViolation('/Volumes/G/another-worktree/target')).toBe(
      'KOHARU_SHARED_TARGET_DIR must equal /Volumes/G/EC-image-koharu/target.',
    )
    expect(sharedTargetViolation('/Volumes/G/EC-image-koharu/target')).toBeUndefined()
  })
})

describe('cargoSweepFailed', () => {
  test('rejects a nonzero exit or cargo-sweep error log', () => {
    expect(cargoSweepFailed(0, '[INFO] Cleaned nothing')).toBe(false)
    expect(cargoSweepFailed(1, '')).toBe(true)
    expect(cargoSweepFailed(0, '[ERROR] Failed to clean target')).toBe(true)
  })
})
