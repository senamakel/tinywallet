# Previous-release x402 writer fixture

`v0.24.1-payment.json` is a stream row emitted by the public
`tinywallet_x402::ledger::PaymentLedger::record_payment` method from the
TinyWallet `v0.24.1` release (`c779230db3008934f97a61f8bdd297351ce66685`).
That release's TSD dependency is `v1.0.0`
(`141fd9ed0ad5d14a8fb5a1b3e837a719e81ac08d`); its TinyBus submodule is
`1f33d1be1901a2d44bfe674cd7cf294cf3848821`. The generator source below is
compiled and run only in that release checkout. The current regression loads
the resulting stream row into the current testkit backend, opens the real
ledger, checks typed readback, and appends a new current-format row. The
separately seeded `x402_budget` and `x402.payments` cases in `schema_tests.rs`
are domain migration tests, not data claimed to have been emitted by this
release writer.

From the native TinyWallet repository root, regenerate it with:

```sh
repo="$PWD"
source="$repo/target/legacy-v0.24.1-source"
mkdir -p "$source/vendor/tinybus" "$source/crates/tinywallet-x402/examples" "$repo/target/compiler-scratch"
git archive c779230db3008934f97a61f8bdd297351ce66685 | tar -x -C "$source"
git -C "$repo/vendor/tinybus" archive 1f33d1be1901a2d44bfe674cd7cf294cf3848821 | tar -x -C "$source/vendor/tinybus"
cp "$repo/crates/tinywallet-x402/src/ledger/fixtures/generate-v0.24.1.rs" "$source/crates/tinywallet-x402/examples/generate_previous_release_fixture.rs"
TMPDIR="$repo/target/compiler-scratch" CARGO_BUILD_JOBS=1 cargo run --locked --manifest-path "$source/Cargo.toml" -p tinywallet-x402 --features ledger --example generate_previous_release_fixture -- "$repo/crates/tinywallet-x402/src/ledger/fixtures/v0.24.1-payment.json"
```

The archived `Cargo.lock` pins TSD v1.0.0 to the commit above; the generator
does not use a local TSD checkout. The TinyBus directory is restored at the
release's exact gitlink even though the ledger-only generator does not enable
the module adapter.

The checked-in output SHA-256 is
`c154765b15e78d7e6e136b8dab6baf3169ab243c2e28bce1f81986f090180e96`.
