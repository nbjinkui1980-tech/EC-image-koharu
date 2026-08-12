#!/bin/sh
set -eu

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)"
hooks_dir="${repo_root}/.githooks"
template_hook="${hooks_dir}/post-checkout"
common_dir="$(git -C "${repo_root}" rev-parse --path-format=absolute --git-common-dir)"
active_hook="${common_dir}/hooks/post-checkout"

resolve_hooks_dir() {
  case "$1" in
    /*) hooks_candidate="$1" ;;
    *) hooks_candidate="${repo_root}/$1" ;;
  esac
  CDPATH= cd -- "${hooks_candidate}" 2>/dev/null && pwd -P
}

configured_hooks="$(git -C "${repo_root}" config --path --get core.hooksPath || true)"
if [ -n "${configured_hooks}" ]; then
  resolved_hooks="$(resolve_hooks_dir "${configured_hooks}" || true)"
  if [ "${resolved_hooks}" != "${hooks_dir}" ]; then
    printf 'Koharu hooks: core.hooksPath is already set to %s; refusing to replace it.\n' \
      "${configured_hooks}" >&2
    exit 1
  fi
  git -C "${repo_root}" config --unset-all core.hooksPath
fi

if [ -e "${active_hook}" ] && ! /usr/bin/cmp -s "${template_hook}" "${active_hook}"; then
  if ! /usr/bin/grep -Fq '# koharu-managed-storage-hook' "${active_hook}"; then
    printf 'Koharu hooks: %s exists and is not managed by Koharu; refusing to overwrite it.\n' \
      "${active_hook}" >&2
    exit 1
  fi
fi

/bin/mkdir -p "${common_dir}/hooks"
/usr/bin/install -m 0755 "${template_hook}" "${active_hook}"
(
  cd "${repo_root}"
  KOHARU_EXTERNAL_VOLUME=/Volumes/G \
    KOHARU_EXTERNAL_TARGET_ROOT=/Volumes/G/EC-image-koharu \
    "${active_hook}"
)

if git -C "${repo_root}" config --get core.hooksPath >/dev/null 2>&1; then
  printf 'Koharu hooks: core.hooksPath must be unset after installation.\n' >&2
  exit 1
fi
[ -x "${active_hook}" ]
/usr/bin/cmp -s "${template_hook}" "${active_hook}"
[ "$(/usr/bin/readlink "${repo_root}/target")" = '/Volumes/G/EC-image-koharu/target' ]
/usr/bin/grep -Fqx 'CARGO_TARGET_DIR=/Volumes/G/EC-image-koharu/target' "${repo_root}/.env"
/usr/bin/grep -Fqx 'KOHARU_SHARED_TARGET_DIR=/Volumes/G/EC-image-koharu/target' "${repo_root}/.env"
/usr/bin/grep -Fqx 'KOHARU_TMPDIR=/Volumes/G/EC-image-koharu/tmp' "${repo_root}/.env"

printf 'Koharu hook installed: %s\n' "${active_hook}"
