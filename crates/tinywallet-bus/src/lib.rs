//! Everything about `TinyWallet` that a host needs and a wallet backend does
//! not: the wire contract that crosses the `TinyBus` boundary, the member names
//! that carry it, and the pure rules a host still runs itself.
//!
//! A host loads the `tinywallet-module` dynamic library but cannot import Rust
//! items from that binary. This crate is the ordinary library that supplies its
//! call vocabulary — interface name, object path, member names, request and
//! response types, and the compatibility rule for that vocabulary.
//!
//! It is deliberately transport-free: no `TinyBus`, no runtime, no HTTP client,
//! and above all no chain library. Key custody, derivation, transaction
//! building and signing, and broadcast are the module's, and taking this crate
//! links none of them — no `bitcoin`, no `secp256k1` C build, no `ethers-core`,
//! no BIP-39 implementation.
//!
//! # Why this crate holds logic and not only types
//!
//! A pure rule belongs here when the host genuinely runs it synchronously and
//! paying a bus round trip for it would be absurd. Four do:
//!
//! - [`address`] — validating an address *before* a spec is sent. A bad address
//!   caught here is a rejected input; caught in the module it is a failed call.
//! - [`eip712`] — hashing typed data for the x402 payment path. Keccak over a
//!   fixed byte layout, with no chain client and no bignum behind it.
//! - [`abi`] — ERC-20 `transfer` calldata: keccak over 68 bytes, an *input* to
//!   building a transaction rather than part of building one.
//! - [`tx::tron`] — verifying the txid and contents of what a Tron node handed
//!   back. Tron has the node build the transaction, so a client that signs
//!   blind authorises whatever a compromised endpoint returned; the check has to
//!   happen wherever the decision to sign is made.
//!
//! This is the same carve-out `tinydocs-bus` makes, and it has the same rule
//! behind it: **a crate owns what is the same for every host; the host owns what
//! depends on its own runtime, config, or threat model.** [`rpc::Transport`] is
//! here for that reason too — it models I/O and performs none, because endpoint
//! selection and retry policy are the host's.
//!
//! # Feature flags
//!
//! | Feature | Gates |
//! | --- | --- |
//! | `btc` | Bitcoin addresses |
//! | `evm` | EVM addresses |
//! | `solana` | Solana addresses |
//! | `tron` | Tron addresses, and [`tx`] with `tx-codec` |
//! | `keccak` | EIP-55 checksums for EVM addresses |
//! | `net` | the [`rpc::Transport`] seam |
//! | `asset` | network and token reference data |
//! | `wire` | the host/module wire contract |
//! | `eip712` | EIP-712 typed-data hashing |
//! | `abi` | ERC-20 `transfer` calldata |
//! | `tx-codec` | the Tron protobuf reader and verification half |

mod error;

#[cfg(feature = "abi")]
pub mod abi;
pub mod address;
#[cfg(feature = "asset")]
pub mod asset;
pub mod chain;
#[cfg(feature = "eip712")]
pub mod eip712;
pub mod names;
#[cfg(feature = "net")]
pub mod rpc;
#[cfg(feature = "tx-codec")]
pub mod tx;
pub mod version;
#[cfg(feature = "wire")]
pub mod wire;

pub use chain::Chain;
pub use error::{Error, Result};
pub use names::{BUS_NAME, CONFIDENTIAL_METHODS, METHODS, OBJECT_PATH};
pub use version::{CONTRACT_VERSION, is_compatible};

#[cfg(test)]
mod test;
