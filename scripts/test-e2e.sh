#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$root"
prepare_only=0
if [[ "${1:-}" == "--prepare-only" && "$#" -eq 1 ]]; then
  prepare_only=1
elif [[ "$#" -ne 0 ]]; then
  echo "usage: $0 [--prepare-only]" >&2
  exit 2
fi
if [[ "${RUNNER_OS:-}" == "Windows" ]]; then
  if (( prepare_only )); then
    pwsh -NoProfile -File scripts/test-e2e.ps1 -PrepareOnly
  else
    pwsh -NoProfile -File scripts/test-e2e.ps1
  fi
  exit
fi
target="${CARGO_TARGET_DIR:-$root/target}"
[[ "$target" == /* ]] || target="$root/$target"
mkdir -p "$target"
target="$(cd "$target" && pwd -P)"
[[ "$target" == "$root"/* ]] || { echo "CARGO_TARGET_DIR must stay inside the repository" >&2; exit 2; }
host="$(rustc -vV | sed -n 's/^host: //p')"
case "$host" in
  *-apple-darwin) name="libtinywallet_module.dylib" ;;
  *-linux-gnu) name="libtinywallet_module.so" ;;
  *-windows-*) echo "test-e2e.sh requires a Unix runner" >&2; exit 1 ;;
  *) echo "unsupported E2E host target: $host" >&2; exit 2 ;;
esac

native_test_logs="$target/tinywallet-native-e2e-logs"
if (( ! prepare_only )); then
  rm -rf "$native_test_logs"
fi
mkdir -p "$native_test_logs"
run_counted_test() {
  local expected_count="$1"
  local test_name="$2"
  shift 2
  local log_path="$native_test_logs/$test_name.log"
  if ! "$@" 2>&1 | tee "$log_path"; then
    echo "native E2E target $test_name failed; see $log_path" >&2
    return 1
  fi
  local actual_count
  actual_count="$(sed -nE 's/^test result: ok\. ([0-9]+) passed;.*/\1/p' "$log_path" | tail -n 1)"
  if [[ "$actual_count" != "$expected_count" ]]; then
    echo "native E2E target $test_name ran ${actual_count:-no reported tests}; expected $expected_count" >&2
    return 1
  fi
}

cargo build --locked --release --package tinywallet-module --features e2e-memory-storage
stage="$target/tinywallet-module-e2e"
rm -rf "$stage"
mkdir -p "$stage"
install -m 755 "$target/release/$name" "$stage/$name"
if command -v sha256sum >/dev/null 2>&1; then
  hash="$(sha256sum "$stage/$name" | awk '{print $1}')"
else
  hash="$(shasum -a 256 "$stage/$name" | awk '{print $1}')"
fi
printf '"%s" = "%s"\n' "$name" "$hash" > "$stage/modules.toml"
if (( ! prepare_only )); then
  run_counted_test 2 module_e2e env TINYWALLET_TEST_MODULE="$stage/$name" \
    cargo test --locked --release --package tinywallet-module --test module_e2e -- --ignored
  run_counted_test 1 lifecycle_e2e env TINYWALLET_TEST_MODULE="$stage/$name" \
    cargo test --locked --release --package tinywallet-module --test lifecycle_e2e -- --ignored
fi

# Client E2E uses this exact local build as both a release archive and a
# library-pinned cache entry. The cached-release loader is forbidden to fetch.
# Its fixture-dependent tests run only after this local archive and both hashes
# have been staged; feature and coverage runners use prepare-only mode first.
client_cache="$target/tinywallet-client-e2e-cache"
unattested_dir="$target/tinywallet-client-unattested"
archive_name="tinywallet-module-client-test.tar.gz"
archive_path="$target/$archive_name"
rm -rf "$client_cache" "$unattested_dir" "$archive_path"
mkdir -p "$client_cache" "$unattested_dir"
chmod 700 "$client_cache" "$unattested_dir"
install -m 755 "$stage/$name" "$client_cache/$name"
cp "$stage/modules.toml" "$client_cache/modules.toml"
tar -C "$client_cache" -czf "$archive_path" .
install -m 644 "$archive_path" "$client_cache/$archive_name"
install -m 755 "$stage/$name" "$unattested_dir/$name"
if command -v sha256sum >/dev/null 2>&1; then
  archive_hash="$(sha256sum "$archive_path" | awk '{print $1}')"
else
  archive_hash="$(shasum -a 256 "$archive_path" | awk '{print $1}')"
fi
export TINYWALLET_CLIENT_TEST_CACHE="$client_cache"
export TINYWALLET_CLIENT_TEST_ARCHIVE_NAME="$archive_name"
export TINYWALLET_CLIENT_TEST_ARCHIVE_SHA256="$archive_hash"
export TINYWALLET_CLIENT_TEST_LIBRARY_SHA256="$hash"
export TINYWALLET_CLIENT_UNATTESTED_MODULE="$unattested_dir/$name"
env_file="$target/tinywallet-client-e2e.env"
{
  printf 'export TINYWALLET_CLIENT_TEST_CACHE=%q\n' "$TINYWALLET_CLIENT_TEST_CACHE"
  printf 'export TINYWALLET_CLIENT_TEST_ARCHIVE_NAME=%q\n' "$TINYWALLET_CLIENT_TEST_ARCHIVE_NAME"
  printf 'export TINYWALLET_CLIENT_TEST_ARCHIVE_SHA256=%q\n' "$TINYWALLET_CLIENT_TEST_ARCHIVE_SHA256"
  printf 'export TINYWALLET_CLIENT_TEST_LIBRARY_SHA256=%q\n' "$TINYWALLET_CLIENT_TEST_LIBRARY_SHA256"
  printf 'export TINYWALLET_CLIENT_UNATTESTED_MODULE=%q\n' "$TINYWALLET_CLIENT_UNATTESTED_MODULE"
} > "$env_file"
if (( prepare_only )); then
  echo "prepared native wallet fixture environment at $env_file"
  exit 0
fi

run_counted_test 3 native_e2e cargo test --locked --release --package tinywallet-client \
  --features native-e2e --test native_e2e -- --test-threads=1
run_counted_test 1 attested_errors cargo test --locked --release --package tinywallet-client \
  --features native-e2e --test attested_errors -- --test-threads=1
run_counted_test 1 native_unattested cargo test --locked --release --package tinywallet-client \
  --features native-e2e --test native_unattested -- --test-threads=1
