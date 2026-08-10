//! BIP-32 derivation on secp256k1, shared by Bitcoin, EVM and Tron.
//!
//! All three chains use the same scheme and differ only in what they do with
//! the resulting key, so the walk lives here once rather than three times.
//!
//! Uses `bitcoin`'s `Xpriv`, which is a vetted BIP-32 implementation. Rolling
//! this by hand is possible — it is HMAC-SHA512 plus a scalar addition — but
//! an off-by-one in the hardened-index encoding produces a *valid key for the
//! wrong account*, which is silent, unrecoverable, and exactly the kind of bug
//! not worth risking to avoid a dependency the crate already has.

use std::str::FromStr;

use bitcoin::bip32::{DerivationPath, Xpriv};
use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use bitcoin::Network;

use super::{Error, Result};

/// A secp256k1 key derived at a BIP-32 path.
pub(super) struct Secp256k1Key {
    pub(super) secret: SecretKey,
    pub(super) public: PublicKey,
}

impl Secp256k1Key {
    /// The 65-byte uncompressed SEC1 encoding, `0x04` prefix included.
    ///
    /// EVM and Tron both hash this — minus the prefix byte — with Keccak-256 to
    /// form an address.
    pub(super) fn uncompressed_public(&self) -> [u8; 65] {
        self.public.serialize_uncompressed()
    }
}

/// Walk `path` from the master key for `seed`.
///
/// `Network::Bitcoin` only selects the version bytes of the serialized
/// extended key, which is never serialized here — the derived secret is
/// identical on any network, so this is correct for EVM and Tron too.
pub(super) fn derive(seed: &[u8], path: &str) -> Result<Secp256k1Key> {
    let master = Xpriv::new_master(Network::Bitcoin, seed).map_err(|_| Error::Derivation {
        step: "BIP-32 master key",
    })?;
    let parsed = DerivationPath::from_str(path).map_err(|e| Error::InvalidPath {
        path: path.to_string(),
        reason: e.to_string(),
    })?;
    let secp = Secp256k1::new();
    let child = master
        .derive_priv(&secp, &parsed)
        .map_err(|_| Error::Derivation {
            step: "BIP-32 child key",
        })?;
    let secret = child.private_key;
    let public = secret.public_key(&secp);
    Ok(Secp256k1Key { secret, public })
}
