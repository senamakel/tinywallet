#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
if [[ "${RUNNER_OS:-}" == "Windows" ]]; then
  pwsh -NoProfile -File scripts/test-features.ps1
  exit
fi
cargo test --locked --workspace
cargo test --locked --workspace --all-features
