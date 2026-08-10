# TinyWallet

Agent-friendly multi-chain wallet primitives in Rust.

`tinywallet` owns the parts of wallet handling that are pure: address formats,
their validation, and the conversions between their encodings. Bitcoin, EVM
chains, Solana, and Tron each get a module, and `address::validate` dispatches
across them for chain-generic callers.

```rust
use tinywallet::{address, Chain};

// Chain-generic dispatch.
let addr = address::validate(Chain::Btc, "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4")?;

// Or reach for a chain's own module when you need more than validation.
let hex = address::tron::to_hex("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t")?;
assert!(hex.starts_with("41"));
# Ok::<(), tinywallet::Error>(())
```

## What it does not do

No network access, no RPC endpoints, no key storage, no transaction
broadcasting. Every function here is a deterministic pure function of its
arguments.

That is the seam, not a gap. Endpoint selection, retry policy, and key custody
depend on a host's config, threat model, and runtime — a crate that guessed at
any of them would be wrong for every host that guessed differently. What is
left is the part that is genuinely the same everywhere.

## What validation proves, per chain

Validation answers one question: is this string a well-formed address on this
chain. How much that is worth varies sharply, and it is worth being explicit
because it is easy to assume otherwise:

| Chain | Checksum | A single typo is… |
| --- | --- | --- |
| Bitcoin | yes (base58check / bech32) | caught |
| Tron | yes (base58check) | caught |
| EVM | optional (EIP-55, only if mixed-case) | usually **not** caught |
| Solana | none | **not** caught — it names another valid address |

For EVM, `address::evm::is_checksum_valid` recovers the typo protection when
the caller has a mixed-case address. For Solana there is nothing to recover:
confirm the address out of band.

## Bitcoin has two rules, not one

Which address is acceptable depends on which side of the transaction it sits:

- `btc::validate` — any well-formed mainnet address. Correct for a
  **recipient**: paying to a P2WPKH, P2TR, P2SH, or P2PKH output is the same
  operation.
- `btc::validate_sender` — additionally requires **P2WPKH** (`bc1q…` native
  segwit), the only script type signing is implemented for.

Using the first where the second belongs is the dangerous direction: it accepts
an address that fails much later, at signing time, after a transaction has been
assembled. They are separate functions rather than a boolean flag so that
mistake reads wrong at the call site.

## Feature flags

Every chain is a separate default-on gate, so a host that needs one chain does
not pay for the others' parsers.

| Feature | Default | Gates | Pulls |
| --- | --- | --- | --- |
| `btc` | on | Bitcoin addresses | `bitcoin` |
| `evm` | on | EVM addresses | — (dependency-free) |
| `solana` | on | Solana addresses | `bs58` |
| `tron` | on | Tron addresses | `bs58`, `hex` |
| `keccak` | on | EIP-55 checksums for EVM | `sha3` |

With a chain's gate off, `address::validate` returns `Error::ChainNotCompiled`
for it — a build fact reported honestly, rather than a wrong answer dressed up
as a real one.

## Layout

```text
src/
├── lib.rs              # crate docs + the entire public re-export surface
├── error/              # crate-wide `Error` and `Result<T>`
├── chain/              # the `Chain` enum, ungated
└── address/
    ├── mod.rs          # chain-generic `validate` dispatch
    ├── btc.rs          # + btc/test.rs
    ├── evm.rs          # + evm/test.rs
    ├── solana.rs       # + solana/test.rs
    └── tron.rs         # + tron/test.rs
tests/
└── public_api.rs       # integration tests against the public API only
examples/
└── basic.rs            # compiled and linted in CI
```

## Development

```sh
git submodule update --init --recursive

cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo run --example basic
```

Run the gated builds too — they are the only thing that catches code that
compiles only when a feature is on:

```sh
cargo clippy --all-targets --no-default-features -- -D warnings
for f in btc evm solana tron keccak; do
  cargo check --lib --no-default-features --features "$f"
done
```

## Roadmap

Address handling is the first slice. The natural next ones, in order of how
cleanly they separate from a host:

1. **Key derivation** — BIP39 seeds, BIP32/SLIP-0010 paths, per-chain keypair
   derivation. Pure, and the largest remaining shared surface.
2. **Transaction encoding** — Solana message serialization, TRC20 ABI
   parameters, PSBT construction. Pure, but each needs its chain's type model.

RPC transport, endpoint config, and key custody stay with the host by design
and are not on this list.

## Documentation

- [`AGENTS.md`](AGENTS.md) — repository guidelines for humans and agents
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — how to propose a change
- [`SECURITY.md`](SECURITY.md) — how to report a vulnerability

## License

GPL-3.0-only. See [LICENSE](LICENSE).
