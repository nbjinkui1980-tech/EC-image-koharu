#!/bin/bash
set -eu

root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)"
external_root="/Volumes/G/EC-image-koharu"

reject_override() {
  printf '%s\n' 'Cargo target/build directory overrides are forbidden for Koharu.' >&2
  exit 2
}

toolchain=()
if [[ "${1-}" == +* ]]; then
  toolchain+=("$1")
  shift
fi

cargo_configs=()
global_arguments=()
subcommand=''
while (($#)); do
  argument="$1"
  shift
  case "${argument}" in
    --target-dir | --target-dir=* | --build-dir | --build-dir=*) reject_override ;;
    --config)
      (($#)) || reject_override
      cargo_configs+=(--config "$1")
      shift
      ;;
    --config=*) cargo_configs+=("${argument}") ;;
    --color | --explain | -C | -Z)
      (($#)) || reject_override
      global_arguments+=("${argument}" "$1")
      shift
      ;;
    -*) global_arguments+=("${argument}") ;;
    *)
      subcommand="${argument}"
      break
      ;;
  esac
done

subcommand_arguments=()
case "${subcommand}" in
  b | bench | build | c | check | clippy | d | doc | fix | install | metadata | package | publish | r | run | rustc | rustdoc | t | test)
    while (($#)); do
      argument="$1"
      shift
      case "${argument}" in
        --)
          subcommand_arguments+=(-- "$@")
          break
          ;;
        --target-dir | --target-dir=* | --build-dir | --build-dir=*) reject_override ;;
        --config)
          (($#)) || reject_override
          cargo_configs+=(--config "$1")
          shift
          ;;
        --config=*) cargo_configs+=("${argument}") ;;
        *) subcommand_arguments+=("${argument}") ;;
      esac
    done
    ;;
  *)
    while (($#)); do
      argument="$1"
      shift
      case "${argument}" in
        --)
          subcommand_arguments+=(-- "$@")
          break
          ;;
        --target-dir | --target-dir=* | --build-dir | --build-dir=*) reject_override ;;
        *) subcommand_arguments+=("${argument}") ;;
      esac
    done
    ;;
esac

arguments=()
if ((${#toolchain[@]})); then
  arguments+=("${toolchain[@]}")
fi
if ((${#cargo_configs[@]})); then
  arguments+=("${cargo_configs[@]}")
fi
if ((${#global_arguments[@]})); then
  arguments+=("${global_arguments[@]}")
fi
arguments+=(
  --config "build.target-dir=\"${external_root}/target\""
  --config "build.build-dir=\"${external_root}/target\""
)
if [ -n "${subcommand}" ]; then
  arguments+=("${subcommand}")
fi
if ((${#subcommand_arguments[@]})); then
  arguments+=("${subcommand_arguments[@]}")
fi

export CARGO_TARGET_DIR="${external_root}/target"
export CARGO_BUILD_TARGET_DIR="${external_root}/target"
export CARGO_BUILD_BUILD_DIR="${external_root}/target"
export KOHARU_SHARED_TARGET_DIR="${external_root}/target"
export KOHARU_TMPDIR="${external_root}/tmp"
export TMPDIR="${KOHARU_TMPDIR}"
export TMP="${KOHARU_TMPDIR}"
export TEMP="${KOHARU_TMPDIR}"
export KOHARU_CARGO_GUARD_ACTIVE=1

bun "${root}/scripts/storage.ts" check 1>&2
exec bun "${root}/scripts/dev.ts" cargo "${arguments[@]}"
