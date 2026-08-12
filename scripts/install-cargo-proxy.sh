#!/bin/sh
set -eu

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)"
template_proxy="${repo_root}/scripts/cargo-proxy.sh"
cargo_proxy="/Users/jinkui/.cargo/bin/cargo"
rustup_proxy="/Users/jinkui/.cargo/bin/rustup"
backup_proxy="/Users/jinkui/.cargo/bin/cargo-rustup-proxy"
temp_proxy="${cargo_proxy}.koharu.$$"

[ -x "${rustup_proxy}" ]

if [ -L "${cargo_proxy}" ]; then
  [ "$(/usr/bin/readlink "${cargo_proxy}")" = rustup ] || {
    printf 'Koharu Cargo proxy: %s is an unexpected symlink; refusing to replace it.\n' \
      "${cargo_proxy}" >&2
    exit 1
  }
elif [ -f "${cargo_proxy}" ]; then
  /usr/bin/grep -Fq '# koharu-managed-cargo-proxy' "${cargo_proxy}" || {
    printf 'Koharu Cargo proxy: %s is not managed by Koharu; refusing to replace it.\n' \
      "${cargo_proxy}" >&2
    exit 1
  }
else
  printf 'Koharu Cargo proxy: %s is missing; refusing to guess the rustup installation.\n' \
    "${cargo_proxy}" >&2
  exit 1
fi

if [ -e "${backup_proxy}" ] || [ -L "${backup_proxy}" ]; then
  [ -L "${backup_proxy}" ] && [ "$(/usr/bin/readlink "${backup_proxy}")" = rustup ] || {
    printf 'Koharu Cargo proxy: backup %s is unexpected; refusing to replace it.\n' \
      "${backup_proxy}" >&2
    exit 1
  }
else
  /bin/ln -s rustup "${backup_proxy}"
fi

trap '/bin/rm -f "${temp_proxy}"' EXIT HUP INT TERM
/usr/bin/install -m 0755 "${template_proxy}" "${temp_proxy}"
/bin/mv "${temp_proxy}" "${cargo_proxy}"
trap - EXIT HUP INT TERM

[ -x "${cargo_proxy}" ]
/usr/bin/cmp -s "${template_proxy}" "${cargo_proxy}"
(
  cd /private/tmp
  "${cargo_proxy}" --version >/dev/null
)

printf 'Koharu Cargo proxy installed: %s\nRecovery: /bin/mv %s %s\n' \
  "${cargo_proxy}" "${backup_proxy}" "${cargo_proxy}"
