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
//! It does **not** carry a key, and no method it exports accepts one — see
//! [`service`] for the two-call split that makes that possible.

mod service;

pub use service::{BUS_NAME, OBJECT_PATH};
