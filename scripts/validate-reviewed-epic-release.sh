#!/usr/bin/env bash

set -euo pipefail

if [[ "${GITHUB_REPOSITORY:-}" != "tinyhumansai/tinywallet" ]]; then
  echo "reviewed releases run only in tinyhumansai/tinywallet" >&2
  exit 2
fi

if [[ "${GITHUB_REF:-}" != "refs/heads/epic/8056-modules-wiring" ]]; then
  echo "reviewed releases run only from epic/8056-modules-wiring" >&2
  exit 2
fi

reviewed_pr="${REVIEWED_PR:-}"
if [[ ! "$reviewed_pr" =~ ^[1-9][0-9]*$ ]]; then
  echo "reviewed_pr must be a positive integer" >&2
  exit 2
fi

if [[ -z "${GITHUB_OUTPUT:-}" ]]; then
  echo "GITHUB_OUTPUT is required" >&2
  exit 2
fi

printf 'reviewed_pr=%s\n' "$reviewed_pr" >> "$GITHUB_OUTPUT"
