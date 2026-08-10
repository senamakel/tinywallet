//! Bitcoin key derivation: BIP-32 on secp256k1, P2WPKH address.
//!
//! Produces a native segwit (`bc1q…`) address, matching
//! [`crate::address::btc::validate_sender`] — the only script type this crate's
//! callers can sign for. Deriving a P2PKH or P2SH address here would hand back
//! something that passes recipient validation and then fails at signing time.

use bitcoin::key::{CompressedPublicKey, PrivateKey};
use bitcoin::{Address, Network};

use super::{bip32, seed_from_mnemonic, DerivedKey, Error, Result};
use crate::chain::Chain;

/// Derive the Bitcoin signing key and P2WPKH address for `path`.
pub(super) fn derive(mnemonic: &str, path: &str) -> Result<DerivedKey> {
    let seed = seed_from_mnemonic(mnemonic)?;
    let key = bip32::derive(&seed, path)?;

    let private = PrivateKey::new(key.secret, Network::Bitcoin);
    let compressed =
        CompressedPublicKey::from_private_key(&bitcoin::secp256k1::Secp256k1::new(), &private)
            .map_err(|_| Error::Derivation {
                step: "BTC compressed public key",
            })?;
    let address = Address::p2wpkh(&compressed, Network::Bitcoin).to_string();

    Ok(DerivedKey::new(
        Chain::Btc,
        address,
        key.secret.secret_bytes().to_vec(),
    ))
}
