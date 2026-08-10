//! Bitcoin address validation.
//!
//! Two functions, because Bitcoin has two different answers depending on which
//! side of a transaction the address sits on:
//!
//! - [`validate`] — any well-formed mainnet address. Correct for a
//!   **recipient**: we do not care which address type they prefer, because
//!   paying to a P2WPKH, P2TR, P2SH or P2PKH output is the same operation.
//! - [`validate_sender`] — additionally requires **P2WPKH** (`bc1q…` native
//!   segwit). Correct for a **sender**, because that is the only script type
//!   this crate's family of signing paths knows how to spend.
//!
//! Calling [`validate`] where [`validate_sender`] belongs is the dangerous
//! direction: it accepts an address that will fail much later, at signing
//! time, after a transaction has been assembled. The two are separate
//! functions rather than a boolean flag so that mistake reads wrong at the
//! call site.

use std::str::FromStr;

use bitcoin::{Address, Network};

use crate::chain::Chain;
use crate::{Error, Result};

/// Validate a Bitcoin **mainnet** address of any type, returning it trimmed.
///
/// Use this for transaction recipients.
///
/// # Errors
///
/// - [`Error::EmptyAddress`] if `address` is empty or all whitespace.
/// - [`Error::InvalidAddress`] if it does not parse as a Bitcoin address.
/// - [`Error::WrongNetwork`] if it parses but belongs to testnet, signet, or
///   regtest.
///
/// # Examples
///
/// ```
/// use tinywallet::address::btc;
///
/// // Native segwit, wrapped segwit, legacy, and taproot are all accepted.
/// assert!(btc::validate("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").is_ok());
/// assert!(btc::validate("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2").is_ok());
///
/// // A testnet address is well-formed but on the wrong network.
/// assert!(btc::validate("tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx").is_err());
/// ```
pub fn validate(address: &str) -> Result<String> {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return Err(Error::EmptyAddress { chain: Chain::Btc });
    }

    Address::from_str(trimmed)
        .map_err(|e| Error::InvalidAddress {
            chain: Chain::Btc,
            address: trimmed.to_string(),
            reason: e.to_string(),
        })?
        .require_network(Network::Bitcoin)
        .map_err(|e| Error::WrongNetwork {
            chain: Chain::Btc,
            address: trimmed.to_string(),
            expected: "mainnet".to_string(),
            reason: e.to_string(),
        })?;

    Ok(trimmed.to_string())
}

/// Validate a Bitcoin address usable as a **sender**, returning it trimmed.
///
/// Everything [`validate`] requires, plus the address must be P2WPKH — native
/// segwit, the `bc1q…` form. Signing is only implemented for that script type,
/// so any other type would fail later with a much less obvious error.
///
/// # Errors
///
/// - Everything [`validate`] returns.
/// - [`Error::UnsupportedAddressType`] if the address is well-formed mainnet
///   but not P2WPKH.
///
/// # Examples
///
/// ```
/// use tinywallet::address::btc;
///
/// // Native segwit: usable as a sender.
/// assert!(btc::validate_sender("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").is_ok());
///
/// // A legacy address is a fine recipient but cannot be signed for here.
/// assert!(btc::validate("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2").is_ok());
/// assert!(btc::validate_sender("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2").is_err());
/// ```
pub fn validate_sender(address: &str) -> Result<String> {
    let trimmed = validate(address)?;

    // `validate` already proved this parses and is mainnet, so `assume_checked`
    // cannot mask a network mismatch here.
    let parsed = Address::from_str(&trimmed)
        .map_err(|e| Error::InvalidAddress {
            chain: Chain::Btc,
            address: trimmed.clone(),
            reason: e.to_string(),
        })?
        .assume_checked();

    if !parsed.script_pubkey().is_p2wpkh() {
        return Err(Error::UnsupportedAddressType {
            chain: Chain::Btc,
            address: trimmed,
            reason: "only P2WPKH (bc1q… native segwit) can be signed for".to_string(),
        });
    }
    Ok(trimmed)
}

#[cfg(test)]
mod test;
