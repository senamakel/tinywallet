//! Loadable `TinyBus` module adapter for `TinyWallet`.
//!
//! This private workspace crate keeps the vendored `TinyBus` dependency out of
//! the independently published `tinywallet` crate. Its `cdylib` output is the
//! target-specific binary distributed in GitHub releases.
//!
//! What it carries is the point: `bitcoin` and its native `secp256k1` build,
//! plus every chain's transaction encoder. A host that loads this module runs
//! the wallet without linking any of them.
//!
//! Public construction and quote requests carry no secret. Confidential
//! signing and payment requests are restricted to attested module channels as
//! documented in `docs/specs/tinybus-module.md`; the interface itself lives in
//! the private `service` module, whose docs are visible with
//! `cargo doc --document-private-items`.

mod service;

#[cfg(feature = "static-link")]
pub use service::linked_module;
pub use service::{BUS_NAME, HOST_INTERFACE, HOST_OBJECT_PATH, OBJECT_PATH};
/// Pure collection declarations for the host; migration behavior stays in the
/// x402 implementation.
pub use tinywallet_bus::storage::collections as storage_collections;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
