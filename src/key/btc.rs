//! Bitcoin key derivation: BIP-32 on secp256k1, P2WPKH address.
//!
//! Produces a native segwit (`bc1q…`) address, matching
//! [`crate::address::btc::validate_sender`] — the only script type this crate's
//! callers can sign for. Deriving a P2PKH or P2SH address here would hand back
//! something that passes recipient validation and then fails at signing time.

use bitcoin::key::{CompressedPublicKey, PrivateKey};
use bitcoin::{Address, Network};

use super::{DerivedKey, Error, Result, bip32, seed_from_mnemonic};
use crate::chain::Chain;

/// Derive the Bitcoin signing key and P2WPKH address for `path`.
pub(super) fn derive(mnemonic: &str, path: &str) -> Result<DerivedKey> {
    let seed = seed_from_mnemonic(mnemonic)?;
    let key = bip32::derive(&seed, path)?;

    let private = PrivateKey::new(key.secret, Network::Bitcoin);
    let compressed = map_compressed_public_key(
        CompressedPublicKey::from_private_key(&bitcoin::secp256k1::Secp256k1::new(), &private),
    )?;
    let address = Address::p2wpkh(&compressed, Network::Bitcoin).to_string();

    Ok(DerivedKey::new(
        Chain::Btc,
        address,
        key.secret.secret_bytes().to_vec(),
    ))
}

/// Collapse an invalid compressed-key conversion into the crate's error type.
pub(super) fn map_compressed_public_key(
    result: std::result::Result<CompressedPublicKey, bitcoin::key::UncompressedPublicKeyError>,
) -> Result<CompressedPublicKey> {
    result.map_err(|_| Error::Derivation {
        step: "BTC compressed public key",
    })
}
