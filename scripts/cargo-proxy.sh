#!/bin/bash
# koharu-managed-cargo-proxy
set -eu

koharu_common_dir="/Users/jinkui/ec-image-Koharu/EC-image-koharu/.git"
koharu_guard="/Users/jinkui/ec-image-Koharu/EC-image-koharu/scripts/cargo-command.sh"
rustup_proxy="/Users/jinkui/.cargo/bin/rustup"

if [ "${KOHARU_CARGO_GUARD_ACTIVE-}" != 1 ]; then
  current_common_dir="$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
  if [ "${current_common_dir}" = "${koharu_common_dir}" ]; then
    exec "${koharu_guard}" "$@"
  fi
fi

exec -a cargo "${rustup_proxy}" "$@"
