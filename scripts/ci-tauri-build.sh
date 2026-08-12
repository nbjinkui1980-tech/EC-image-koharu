#!/usr/bin/env bash
# Wrapper for tauri-apps/tauri-action that captures build output so that
# the failing tail can be surfaced as a GitHub Actions annotation when logs
# are otherwise not readable from the outside.
set -o pipefail

npx @tauri-apps/cli "$@" 2>&1 | tee tauri-build.log
exit "${PIPESTATUS[0]}"
