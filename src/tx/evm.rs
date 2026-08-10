//! Ethereum legacy (type-0) transaction building and EIP-155 signing.
//!
//! Legacy rather than EIP-1559 because it is accepted by every EVM network in
//! the catalog, including the ones that never adopted 1559 — a type-2
//! transaction is rejected outright on those, whereas a legacy one is
//! universally valid.
//!
//! ## EIP-155 is replay protection, and it is the easy thing to get wrong
//!
//! Before EIP-155 a signed transaction was valid on *every* EVM chain, so a
//! transfer on one network could be replayed byte-for-byte on another and
//! spend real funds. EIP-155 fixes that by folding the chain id into the
//! signature two ways at once:
//!
//! 1. the payload that gets hashed ends with `chain_id, 0, 0` instead of
//!    stopping after `data`, and
//! 2. the resulting `v` is `recovery_id + chain_id * 2 + 35` rather than
//!    `recovery_id + 27`.
//!
//! Both are required. Doing only the second produces a signature that
//! recovers to the wrong address; doing only the first produces one that a
//! node rejects. Neither mistake is visible without a known-good vector to
//! compare against, which is why this module's tests pin the published
//! EIP-155 example byte-for-byte.

use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use sha3::{Digest, Keccak256};

use super::rlp;
use super::{Error, Result};

/// An unsigned legacy transaction.
///
/// Every numeric field is in base units — wei for `value` and `gas_price`, a
/// plain count for `gas_limit` — for the reasons in [`crate::client`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyTransaction {
    /// Sender's next transaction count.
    pub nonce: u128,
    /// Price per unit of gas, in wei.
    pub gas_price: u128,
    /// Maximum gas this transaction may consume.
    pub gas_limit: u128,
    /// Recipient address, `0x`-prefixed hex.
    ///
    /// `None` means contract creation, which this crate does not otherwise
    /// support but which the encoding represents as an empty `to` field.
    pub to: Option<String>,
    /// Amount to transfer, in wei.
    pub value: u128,
    /// Calldata. Empty for a plain transfer; an ABI-encoded call for a token
    /// transfer.
    pub data: Vec<u8>,
    /// EIP-155 chain id. See the module docs — this is what stops the signed
    /// transaction being replayable on another network.
    pub chain_id: u64,
}

impl LegacyTransaction {
    /// The nine RLP items common to the signing payload and the signed
    /// transaction, up to and including `data`.
    fn base_items(&self) -> Result<Vec<Vec<u8>>> {
        Ok(vec![
            rlp::encode_uint(self.nonce),
            rlp::encode_uint(self.gas_price),
            rlp::encode_uint(self.gas_limit),
            rlp::encode_bytes(&self.to_bytes()?),
            rlp::encode_uint(self.value),
            rlp::encode_bytes(&self.data),
        ])
    }

    /// The recipient as 20 raw bytes, or empty for contract creation.
    fn to_bytes(&self) -> Result<Vec<u8>> {
        let Some(to) = self.to.as_deref() else {
            return Ok(Vec::new());
        };
        let validated = crate::address::evm::validate(to).map_err(Error::Address)?;
        let body = validated.strip_prefix("0x").unwrap_or(&validated);
        decode_hex(body).ok_or_else(|| Error::InvalidField {
            field: "to",
            reason: format!("not hex: {to}"),
        })
    }

    /// The exact bytes that get Keccak-hashed and signed.
    ///
    /// Ends with `chain_id, 0, 0` per EIP-155 — the first half of the replay
    /// protection described in the module docs.
    ///
    /// # Errors
    ///
    /// [`Error::Address`] if `to` is not a valid EVM address.
    pub fn signing_payload(&self) -> Result<Vec<u8>> {
        let mut items = self.base_items()?;
        items.push(rlp::encode_uint(u128::from(self.chain_id)));
        items.push(rlp::encode_uint(0));
        items.push(rlp::encode_uint(0));
        Ok(rlp::encode_list(&items))
    }

    /// Sign with `secret_key` and return the raw transaction bytes ready for
    /// `eth_sendRawTransaction`.
    ///
    /// # Errors
    ///
    /// - [`Error::Address`] if `to` is invalid.
    /// - [`Error::Signing`] if the key is not a valid secp256k1 scalar.
    pub fn sign(&self, secret_key: &[u8]) -> Result<Vec<u8>> {
        let secret = SecretKey::from_slice(secret_key).map_err(|_| Error::Signing {
            reason: "not a valid secp256k1 secret key".to_string(),
        })?;

        let digest = Keccak256::digest(self.signing_payload()?);
        let message = Message::from_digest(digest.into());

        let secp = Secp256k1::signing_only();
        // Recoverable, because an Ethereum signature carries the recovery id
        // in `v` rather than shipping the public key.
        let signature = secp.sign_ecdsa_recoverable(&message, &secret);
        let (recovery_id, bytes) = signature.serialize_compact();

        // The second half of EIP-155: v = recovery + chain_id * 2 + 35.
        // `RecoveryId` is 0..=3, so the conversion cannot lose information.
        let recovery = u64::try_from(recovery_id.to_i32()).map_err(|_| Error::Signing {
            reason: "negative recovery id".to_string(),
        })?;
        let v = recovery
            .checked_add(
                self.chain_id
                    .checked_mul(2)
                    .and_then(|d| d.checked_add(35))
                    .ok_or_else(|| Error::InvalidField {
                        field: "chain_id",
                        reason: format!("{} overflows EIP-155 v", self.chain_id),
                    })?,
            )
            .ok_or_else(|| Error::InvalidField {
                field: "chain_id",
                reason: "overflows EIP-155 v".to_string(),
            })?;

        let mut items = self.base_items()?;
        items.push(rlp::encode_uint(u128::from(v)));
        items.push(rlp::encode_uint_bytes(&bytes[..32]));
        items.push(rlp::encode_uint_bytes(&bytes[32..]));
        Ok(rlp::encode_list(&items))
    }

    /// The transaction hash a node will report, given the signed bytes.
    ///
    /// Keccak-256 of the *signed* encoding, `0x`-prefixed — not of the signing
    /// payload, which hashes to a different value.
    #[must_use]
    pub fn hash_of(signed: &[u8]) -> String {
        let digest = Keccak256::digest(signed);
        let mut out = String::with_capacity(66);
        out.push_str("0x");
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
        }
        out
    }
}

/// Decode an even-length hex string into bytes.
fn decode_hex(body: &str) -> Option<Vec<u8>> {
    if body.len() % 2 != 0 {
        return None;
    }
    (0..body.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&body[i..i + 2], 16).ok())
        .collect()
}

/// ABI-encode an ERC-20 `transfer(address,uint256)` call.
///
/// The four-byte selector is the first four bytes of
/// `keccak256("transfer(address,uint256)")`, followed by two 32-byte
/// big-endian words: the recipient left-padded to 32 bytes, then the amount.
///
/// # Errors
///
/// [`Error::Address`] if `to` is not a valid EVM address.
pub fn encode_erc20_transfer(to: &str, amount: u128) -> Result<Vec<u8>> {
    /// `keccak256("transfer(address,uint256)")[..4]`.
    const SELECTOR: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];

    let validated = crate::address::evm::validate(to).map_err(Error::Address)?;
    let body = validated.strip_prefix("0x").unwrap_or(&validated);
    let address = decode_hex(body).ok_or_else(|| Error::InvalidField {
        field: "to",
        reason: format!("not hex: {to}"),
    })?;

    let mut out = Vec::with_capacity(4 + 64);
    out.extend_from_slice(&SELECTOR);
    // Address, left-padded into a 32-byte word.
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&address);
    // Amount, big-endian in a 32-byte word. u128 is 16 bytes, so pad 16.
    out.extend_from_slice(&[0u8; 16]);
    out.extend_from_slice(&amount.to_be_bytes());
    Ok(out)
}
