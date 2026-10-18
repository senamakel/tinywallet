#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
if [[ "${RUNNER_OS:-}" == "Windows" ]]; then
  pwsh -NoProfile -File scripts/test-features.ps1
  exit
fi
cargo test --locked --workspace
bash scripts/test-e2e.sh --prepare-only
target="${CARGO_TARGET_DIR:-$root/target}"
[[ "$target" == /* ]] || target="$root/$target"
target="$(cd "$target" && pwd -P)"
# The all-feature suite executes the native client targets with a real staged
# module; missing or stale fixture state must fail, never fake or skip them.
source "$target/tinywallet-client-e2e.env"
cargo test --locked --workspace --all-features
