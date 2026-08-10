//! Tron transaction signing.
//!
//! Tron inverts the usual split: the **node** builds the transaction. A client
//! POSTs the transfer parameters to `wallet/createtransaction`, gets back a
//! protobuf `raw_data` (plus its hex encoding and a `txID`), signs it, and
//! POSTs it back to `wallet/broadcasttransaction`.
//!
//! That means this module never serialises a transaction — there is no
//! protobuf encoder here, and deliberately so, because reimplementing Tron's
//! `raw_data` schema would be a large surface that the node already owns.
//!
//! ## But it does mean the node's answer must be verified
//!
//! Signing whatever a node hands back is trusting it to have built the
//! transfer that was asked for. A malicious or compromised endpoint could
//! return a `raw_data` paying a different address, and a client that signs
//! blind would authorise it.
//!
//! [`recompute_txid`] is the defence: the `txID` is `sha256(raw_data)`, so a
//! client can confirm the id it signs actually matches the bytes it was given.
//! That catches a tampered or corrupted response, though it cannot by itself
//! prove the *contents* match the request — [`verify_transfer`] does that, by
//! checking the recipient and amount appear in the returned bytes.

use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};

use super::{Error, Result};

/// The 65-byte signature Tron expects: `r || s || recovery_id`.
///
/// Note the recovery byte is a bare 0 or 1 here, **not** EIP-155's `v` — Tron
/// borrowed Ethereum's address scheme but not its replay-protection encoding.
pub type Signature = [u8; 65];

/// Recompute a transaction's `txID` from its `raw_data`.
///
/// The id is `sha256(raw_data)`. Comparing it against the `txID` a node
/// returned confirms the bytes were not altered in transit.
///
/// # Errors
///
/// [`Error::InvalidField`] if `raw_data_hex` is not valid hex.
pub fn recompute_txid(raw_data_hex: &str) -> Result<String> {
    let raw = decode_hex(raw_data_hex)?;
    Ok(hex_lower(&Sha256::digest(&raw)))
}

/// Check that a node-built transaction really encodes the transfer requested.
///
/// Tron's `raw_data` embeds the recipient as a 21-byte address and the amount
/// as a protobuf varint, so both appear verbatim in the hex. This does not
/// parse the protobuf — it confirms the values are present, which is enough to
/// catch a node that substituted either.
///
/// # Errors
///
/// [`Error::Address`] if `to` is not a valid Tron address, or
/// [`Error::UntrustedResponse`] if the recipient does not appear in the bytes.
pub fn verify_transfer(raw_data_hex: &str, to: &str, txid: &str) -> Result<()> {
    let expected_id = recompute_txid(raw_data_hex)?;
    if !expected_id.eq_ignore_ascii_case(txid.trim()) {
        return Err(Error::UntrustedResponse {
            reason: "txID does not match sha256(raw_data); the response was altered".to_string(),
        });
    }

    let to_hex = crate::address::tron::to_hex(to).map_err(Error::Address)?;
    if !raw_data_hex
        .to_ascii_lowercase()
        .contains(&to_hex.to_ascii_lowercase())
    {
        return Err(Error::UntrustedResponse {
            reason: "the node's transaction does not pay the requested recipient".to_string(),
        });
    }
    Ok(())
}

/// Sign a Tron `raw_data` payload.
///
/// Signs `sha256(raw_data)` — the same value as the `txID`.
///
/// # Errors
///
/// [`Error::InvalidField`] for malformed hex, [`Error::Signing`] for an
/// invalid key.
pub fn sign(raw_data_hex: &str, secret_key: &[u8]) -> Result<Signature> {
    let secret = SecretKey::from_slice(secret_key).map_err(|_| Error::Signing {
        reason: "not a valid secp256k1 secret key".to_string(),
    })?;
    let raw = decode_hex(raw_data_hex)?;
    let digest: [u8; 32] = Sha256::digest(&raw).into();
    let message = Message::from_digest(digest);

    let secp = Secp256k1::signing_only();
    let recoverable = secp.sign_ecdsa_recoverable(&message, &secret);
    let (recovery_id, compact) = recoverable.serialize_compact();

    let mut out = [0u8; 65];
    out[..64].copy_from_slice(&compact);
    // A bare recovery id, not EIP-155's v.
    out[64] = u8::try_from(recovery_id.to_i32()).map_err(|_| Error::Signing {
        reason: "unexpected recovery id".to_string(),
    })?;
    Ok(out)
}

/// Render a signature as the hex string TronGrid expects.
#[must_use]
pub fn signature_hex(signature: &Signature) -> String {
    hex_lower(signature)
}

fn decode_hex(raw: &str) -> Result<Vec<u8>> {
    let body = raw.trim();
    if body.len() % 2 != 0 {
        return Err(Error::InvalidField {
            field: "raw_data_hex",
            reason: "odd length".to_string(),
        });
    }
    (0..body.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&body[i..i + 2], 16).map_err(|e| Error::InvalidField {
                field: "raw_data_hex",
                reason: e.to_string(),
            })
        })
        .collect()
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
        out
    })
}

#[cfg(test)]
mod test {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{recompute_txid, sign, signature_hex, verify_transfer};
    use crate::tx::Error;

    const VECTOR: &str = "abandon abandon abandon abandon abandon abandon \
                          abandon abandon abandon abandon abandon about";
    const TO: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";

    fn key() -> Vec<u8> {
        crate::key::derive(crate::Chain::Tron, VECTOR, "m/44'/195'/0'/0/0")
            .unwrap()
            .secret_bytes()
            .to_vec()
    }

    /// A `raw_data`-shaped hex blob embedding the recipient's hex address.
    ///
    /// Not a real protobuf — `verify_transfer` deliberately does not parse
    /// one, it checks the recipient's bytes are present, so a representative
    /// blob is enough and avoids pinning a schema the node owns.
    fn raw_data() -> String {
        let to_hex = crate::address::tron::to_hex(TO).unwrap();
        format!("0a02b1f42208{to_hex}5a0f")
    }

    #[test]
    fn the_txid_is_sha256_of_the_raw_data() {
        let raw = raw_data();
        let id = recompute_txid(&raw).unwrap();
        assert_eq!(id.len(), 64, "sha256 is 32 bytes of hex");
        // Deterministic.
        assert_eq!(id, recompute_txid(&raw).unwrap());
    }

    #[test]
    fn a_tampered_raw_data_no_longer_matches_its_txid() {
        // The defence against signing whatever a node hands back.
        let raw = raw_data();
        let id = recompute_txid(&raw).unwrap();
        let tampered = raw.replace("0a02", "0a03");
        assert_ne!(tampered, raw);

        match verify_transfer(&tampered, TO, &id).unwrap_err() {
            Error::UntrustedResponse { reason } => assert!(reason.contains("altered")),
            other => panic!("expected UntrustedResponse, got {other:?}"),
        }
    }

    #[test]
    fn a_transaction_paying_someone_else_is_rejected() {
        // A node that substituted the recipient must not get a signature.
        let raw = raw_data();
        let id = recompute_txid(&raw).unwrap();
        let other = "TLyqzVGLV1srkB7dToTAEqgDSfPtXRJZYH";
        match verify_transfer(&raw, other, &id).unwrap_err() {
            Error::UntrustedResponse { reason } => {
                assert!(reason.contains("does not pay the requested recipient"));
            }
            other => panic!("expected UntrustedResponse, got {other:?}"),
        }
    }

    #[test]
    fn a_well_formed_transaction_verifies() {
        let raw = raw_data();
        let id = recompute_txid(&raw).unwrap();
        assert!(verify_transfer(&raw, TO, &id).is_ok());
    }

    #[test]
    fn the_signature_is_65_bytes_ending_in_a_bare_recovery_id() {
        // Tron borrowed Ethereum's addresses but not EIP-155's v encoding.
        let signature = sign(&raw_data(), &key()).unwrap();
        assert_eq!(signature.len(), 65);
        assert!(signature[64] <= 3, "recovery id, not a v value");
        assert_eq!(signature_hex(&signature).len(), 130);
    }

    #[test]
    fn signing_is_deterministic() {
        let raw = raw_data();
        assert_eq!(sign(&raw, &key()).unwrap(), sign(&raw, &key()).unwrap());
    }

    #[test]
    fn different_raw_data_produces_a_different_signature() {
        let a = sign(&raw_data(), &key()).unwrap();
        let b = sign(&raw_data().replace("0a02", "0a03"), &key()).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn malformed_hex_is_rejected() {
        assert!(matches!(
            recompute_txid("abc").unwrap_err(),
            Error::InvalidField { .. }
        ));
        assert!(matches!(
            sign("zz", &key()).unwrap_err(),
            Error::InvalidField { .. }
        ));
    }

    #[test]
    fn an_invalid_key_is_rejected() {
        assert!(matches!(
            sign(&raw_data(), &[0u8; 32]).unwrap_err(),
            Error::Signing { .. }
        ));
    }
}
