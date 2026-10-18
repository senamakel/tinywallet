#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
report="target/coverage.json"
bash scripts/test-e2e.sh --prepare-only
target="${CARGO_TARGET_DIR:-$root/target}"
[[ "$target" == /* ]] || target="$root/$target"
target="$(cd "$target" && pwd -P)"
# Instrument the workspace's real attested client tests against a staged local
# module so their production call paths contribute to the unchanged 90% floor.
source "$target/tinywallet-client-e2e.env"
cargo llvm-cov --workspace --all-features --all-targets --json --output-path "$report"
below="$(jq -r --arg root "$PWD" '
  .data[0].files[]
  | select(.filename | contains("/src/"))
  | select(.filename | startswith($root + "/vendor/") | not)
  | select(.summary.lines.percent < 90)
  | "\(.filename): \(.summary.lines.percent)%"
' "$report")"
if [[ -n "$below" ]]; then
  echo "Files below 90% line coverage:" >&2
  echo "$below" >&2
  jq -r --arg file "$PWD/crates/tinywallet-module/src/service/mod.rs" '
    .data[0].files[]
    | select(.filename == $file)
    | (.segments // [])[]
    | select(.[3] == true and .[2] == 0)
    | .[0]
  ' "$report" | sort -un | paste -sd, - | sed 's/^/Uncovered service lines: /' >&2
  exit 1
fi
