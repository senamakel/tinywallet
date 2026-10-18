$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $root

cargo test --locked --workspace
if ($LASTEXITCODE -ne 0) { throw 'TinyWallet default workspace tests failed' }

# Dot-sourcing prepares the exact local release artifact and leaves its verified
# cache paths in this process environment for the all-features test run.
$nativeSetup = Join-Path $root 'scripts/test-e2e.ps1'
. $nativeSetup -PrepareOnly
cargo test --locked --workspace --all-features
if ($LASTEXITCODE -ne 0) { throw 'TinyWallet all-feature workspace tests failed' }
